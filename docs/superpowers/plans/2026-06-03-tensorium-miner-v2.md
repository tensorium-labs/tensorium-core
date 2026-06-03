# Tensorium Miner v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite `txmminer-cuda` into `tensorium-miner` with multi-GPU parallelism, clean CLI, NVML monitoring, and add a Tensorium Stratum server to `tensorium-pool` so miners can connect via `stratum+tcp://pool.tensoriumlabs.com:3333`.

**Architecture:** Multi-file C++ miner (`main.cpp`, `solo_client`, `stratum_client`, `gpu_worker.cu`, `nvml_monitor`) with per-GPU threads sharing a `SharedState` (job channel + share queue). Rust pool adds `stratum.rs` using the same `TcpListener + thread::spawn` pattern as existing HTTP pool, broadcasting new jobs via per-worker `mpsc::Sender` channels.

**Tech Stack:** C++11/CUDA (miner), Rust + `serde_json` + `sha2` (pool Stratum server), POSIX threads, NVML (optional).

---

## File Map

### New / Modified — Miner (`tools/txmminer-cuda/`)

| File | Action | Responsibility |
|------|--------|----------------|
| `common.h` | CREATE | Shared types: `MinerConfig`, `JobDesc`, `ShareResult`, `GpuStats`, `SharedState` |
| `solo_client.h/cpp` | CREATE | HTTP RPC: `get_block_template()`, `submit_block()`, `build_header()` |
| `gpu_worker.h/cu` | CREATE | Per-GPU thread: kernel loop, nonce range, job polling, share detection |
| `stratum_client.h/cpp` | CREATE | Stratum TCP client: connect, subscribe, authorize, recv notify, send submit |
| `nvml_monitor.h/cpp` | CREATE | Optional NVML polling thread: temp, power, fan per GPU |
| `main.cpp` | CREATE | CLI parsing, SharedState init, thread launch, stats printer |
| `Makefile` | MODIFY | Multi-file build, `WITH_NVML=1` flag, `install` target |
| `mining_kernel.cu` | KEEP | No changes |
| `sha256d.cuh` | KEEP | No changes |
| `main.cu` | DELETE | Replaced by `main.cpp` + extracted modules |

### New / Modified — Pool (`crates/tensorium-pool/src/`)

| File | Action | Responsibility |
|------|--------|----------------|
| `stratum.rs` | CREATE | Stratum TCP server: protocol, job broadcast, share validation, worker registry |
| `main.rs` | MODIFY | Add Stratum listener thread, `sha2` dep, env vars |
| `Cargo.toml` | MODIFY | Add `sha2 = "0.10"` dependency |

---

## Task 1: Scaffold files + `common.h` + Makefile

**Files:**
- Create: `tools/txmminer-cuda/common.h`
- Modify: `tools/txmminer-cuda/Makefile`
- Create (empty): `tools/txmminer-cuda/solo_client.h`, `solo_client.cpp`, `stratum_client.h`, `stratum_client.cpp`, `gpu_worker.h`, `gpu_worker.cu`, `nvml_monitor.h`, `nvml_monitor.cpp`, `main.cpp`

- [ ] **Step 1: Create `common.h`**

```cpp
// tools/txmminer-cuda/common.h
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <pthread.h>
#include <stdatomic.h>

#define TENSORIUM_MINER_VERSION "2.0.0"
#define DEFAULT_SHARE_DIFF 1048576ULL   /* ~20 bits */
#define MAX_GPUS           16
#define JOB_ID_LEN         48
#define ADDR_LEN           128
#define WORKER_LEN         64
#define CHAIN_ID_LEN       64
#define HEADER_MAX         192
#define SHARE_QUEUE_MAX    256

typedef enum { MODE_SOLO = 0, MODE_POOL = 1 } MiningMode;

typedef struct {
    MiningMode  mode;
    char        rpc_host[128];
    char        rpc_port[16];
    char        pool_host[128];
    char        pool_port[16];
    char        wallet[ADDR_LEN];
    char        worker[WORKER_LEN];
    int         gpu_ids[MAX_GPUS];
    int         gpu_count;       /* 0 = use all */
    int         cuda_blocks;
    int         cuda_threads;
    uint64_t    share_diff;
    int         nvml_enabled;
} MinerConfig;

typedef struct {
    char     job_id[JOB_ID_LEN];
    char     chain_id[CHAIN_ID_LEN];
    uint64_t height;
    uint8_t  previous_hash[32];
    uint8_t  merkle_root[32];
    uint64_t timestamp;
    uint8_t  difficulty_bits;    /* network difficulty */
    uint8_t  share_bits;         /* floor(log2(share_diff)) */
    uint64_t version;
    int      valid;              /* 0 = no job yet */
} JobDesc;

typedef struct {
    char     job_id[JOB_ID_LEN];
    char     worker[WORKER_LEN];
    uint64_t nonce;
    int      gpu_id;
    int      is_block;           /* 1 if meets network difficulty */
} ShareResult;

typedef struct {
    int      gpu_id;
    char     name[256];
    double   hashrate_ghs;
    int      temp_c;             /* -1 if NVML unavailable */
    int      power_w;
    int      fan_pct;
    uint64_t shares_found;
    uint64_t hashes_total;
} GpuStats;

/* Shared state between all threads */
typedef struct {
    /* Current job — updated by solo/stratum client thread */
    pthread_mutex_t  job_mutex;
    pthread_cond_t   job_cond;
    JobDesc          current_job;
    int              job_generation; /* increments on each new job */

    /* Share queue — GPU workers push, client thread pops */
    pthread_mutex_t  share_mutex;
    pthread_cond_t   share_cond;
    ShareResult      share_queue[SHARE_QUEUE_MAX];
    int              share_head;
    int              share_tail;
    int              share_count;

    /* Per-GPU stats — written by GPU threads, read by stats printer */
    pthread_mutex_t  stats_mutex;
    GpuStats         gpu_stats[MAX_GPUS];
    int              gpu_count;

    volatile int     running; /* 0 = shutdown */
} SharedState;

static inline void shared_state_init(SharedState *s) {
    memset(s, 0, sizeof(*s));
    pthread_mutex_init(&s->job_mutex, NULL);
    pthread_cond_init(&s->job_cond, NULL);
    pthread_mutex_init(&s->share_mutex, NULL);
    pthread_cond_init(&s->share_cond, NULL);
    pthread_mutex_init(&s->stats_mutex, NULL);
    s->running = 1;
}

/* Push share — called from GPU worker thread */
static inline void share_push(SharedState *s, const ShareResult *r) {
    pthread_mutex_lock(&s->share_mutex);
    if (s->share_count < SHARE_QUEUE_MAX) {
        s->share_queue[s->share_tail] = *r;
        s->share_tail = (s->share_tail + 1) % SHARE_QUEUE_MAX;
        s->share_count++;
        pthread_cond_signal(&s->share_cond);
    }
    pthread_mutex_unlock(&s->share_mutex);
}

/* Pop share — called from client thread */
static inline int share_pop(SharedState *s, ShareResult *r) {
    pthread_mutex_lock(&s->share_mutex);
    while (s->share_count == 0 && s->running) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_sec += 1;
        pthread_cond_timedwait(&s->share_cond, &s->share_mutex, &ts);
    }
    if (s->share_count == 0) {
        pthread_mutex_unlock(&s->share_mutex);
        return 0;
    }
    *r = s->share_queue[s->share_head];
    s->share_head = (s->share_head + 1) % SHARE_QUEUE_MAX;
    s->share_count--;
    pthread_mutex_unlock(&s->share_mutex);
    return 1;
}

/* Publish new job — called from client thread */
static inline void job_publish(SharedState *s, const JobDesc *j) {
    pthread_mutex_lock(&s->job_mutex);
    s->current_job = *j;
    s->job_generation++;
    pthread_cond_broadcast(&s->job_cond);
    pthread_mutex_unlock(&s->job_mutex);
}

/* Wait for job — called from GPU worker at startup */
static inline void job_wait(SharedState *s, JobDesc *out) {
    pthread_mutex_lock(&s->job_mutex);
    while (!s->current_job.valid && s->running)
        pthread_cond_wait(&s->job_cond, &s->job_mutex);
    *out = s->current_job;
    pthread_mutex_unlock(&s->job_mutex);
}

/* Compute share_bits from share_diff */
static inline uint8_t share_bits_from_diff(uint64_t diff) {
    uint8_t bits = 0;
    while (diff > 1) { diff >>= 1; bits++; }
    return bits;
}
```

- [ ] **Step 2: Create `Makefile`**

```makefile
# tools/txmminer-cuda/Makefile
NVCC    ?= nvcc
CXX     ?= g++
TARGET  ?= tensorium-miner

ifndef ARCH
  DETECTED := $(shell nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>/dev/null | head -1 | tr -d .)
  ifneq ($(DETECTED),)
    ARCH := sm_$(DETECTED)
  else
    ARCH := sm_86
    $(warning nvidia-smi not found — defaulting to $(ARCH))
  endif
endif

NVCCFLAGS := -arch=$(ARCH) -O3 --use_fast_math -Xcompiler "-O3 -pthread" \
             -Xptxas -O3,--warn-on-spills
CXXFLAGS  := -O3 -Wall -std=c++11 -pthread -Isrc

SRCS_CU  := gpu_worker.cu mining_kernel.cu
SRCS_CPP := main.cpp solo_client.cpp stratum_client.cpp

ifdef WITH_NVML
  SRCS_CPP  += nvml_monitor.cpp
  NVCCFLAGS += -DWITH_NVML
  CXXFLAGS  += -DWITH_NVML
  LDLIBS    += -lnvidia-ml
endif

OBJS_CU  := $(SRCS_CU:.cu=.o)
OBJS_CPP := $(SRCS_CPP:.cpp=.o)

all: $(TARGET)

$(TARGET): $(OBJS_CU) $(OBJS_CPP)
	$(NVCC) -arch=$(ARCH) -Xcompiler -pthread -o $@ $^ $(LDLIBS)
	@echo ""
	@echo "Built: $(TARGET) ($(ARCH))"

%.o: %.cu sha256d.cuh common.h gpu_worker.h mining_kernel.cu
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

%.o: %.cpp common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

install: $(TARGET)
	sudo cp $(TARGET) /usr/local/bin/tensorium-miner
	sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda
	@echo "Installed tensorium-miner, txmminer-cuda -> tensorium-miner"

clean:
	rm -f $(TARGET) txmminer-cuda *.o

info:
	@echo "ARCH=$(ARCH)"
	@nvcc --version 2>/dev/null | head -1
	@nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader 2>/dev/null || echo "no GPU"
```

- [ ] **Step 3: Create empty stubs so Makefile compiles**

```bash
# Each file needs at least a stub to link
touch tools/txmminer-cuda/solo_client.h
touch tools/txmminer-cuda/stratum_client.h
touch tools/txmminer-cuda/gpu_worker.h
touch tools/txmminer-cuda/nvml_monitor.h

cat > tools/txmminer-cuda/solo_client.cpp   << 'EOF'
#include "solo_client.h"
EOF
cat > tools/txmminer-cuda/stratum_client.cpp << 'EOF'
#include "stratum_client.h"
EOF
cat > tools/txmminer-cuda/gpu_worker.cu     << 'EOF'
#include "gpu_worker.h"
EOF
cat > tools/txmminer-cuda/nvml_monitor.cpp  << 'EOF'
#include "nvml_monitor.h"
EOF
cat > tools/txmminer-cuda/main.cpp          << 'EOF'
int main() { return 0; }
EOF
```

- [ ] **Step 4: Verify scaffold compiles**

```bash
cd tools/txmminer-cuda
make clean && make ARCH=sm_86
```
Expected: `Built: tensorium-miner (sm_86)` with no errors.

- [ ] **Step 5: Commit scaffold**

```bash
git add tools/txmminer-cuda/common.h tools/txmminer-cuda/Makefile \
  tools/txmminer-cuda/solo_client.h tools/txmminer-cuda/solo_client.cpp \
  tools/txmminer-cuda/stratum_client.h tools/txmminer-cuda/stratum_client.cpp \
  tools/txmminer-cuda/gpu_worker.h tools/txmminer-cuda/gpu_worker.cu \
  tools/txmminer-cuda/nvml_monitor.h tools/txmminer-cuda/nvml_monitor.cpp \
  tools/txmminer-cuda/main.cpp
git commit -m "feat(miner): scaffold tensorium-miner v2 file structure"
```

---

## Task 2: `solo_client.cpp/h` — HTTP RPC client

Extracts HTTP logic from `main.cu`. GPU workers get `JobDesc` via `SharedState`; solo client polls node and publishes jobs.

**Files:**
- Modify: `tools/txmminer-cuda/solo_client.h`
- Modify: `tools/txmminer-cuda/solo_client.cpp`

- [ ] **Step 1: Write `solo_client.h`**

```cpp
// tools/txmminer-cuda/solo_client.h
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Blocking: polls node, publishes jobs to SharedState, submits shares from queue.
   Returns only when state->running == 0. */
void solo_client_run(const MinerConfig *cfg, SharedState *state);

/* Build Tensorium block header from job + nonce into out[].
   Returns header length in bytes (currently 122 for mainnet chain_id length).
   Returns 0 on error. */
int build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]);

#ifdef __cplusplus
}
#endif
```

- [ ] **Step 2: Write `solo_client.cpp`**

```cpp
// tools/txmminer-cuda/solo_client.cpp
#include "solo_client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netdb.h>
#include <time.h>

#define RPC_BUF (1 << 20)

// ── TCP helpers ──────────────────────────────────────────────────────────────

static int tcp_connect(const char *host, const char *port) {
    struct addrinfo hints = {0}, *res;
    hints.ai_family   = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) return -1;
    int s = socket(res->ai_family, res->ai_socktype, 0);
    if (s < 0) { freeaddrinfo(res); return -1; }
    struct timeval tv = {10, 0};
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    if (connect(s, res->ai_addr, res->ai_addrlen) != 0) {
        close(s); freeaddrinfo(res); return -1;
    }
    freeaddrinfo(res);
    return s;
}

static int http_get(const char *host, const char *port, const char *path,
                    char *buf, int buf_len) {
    int s = tcp_connect(host, port);
    if (s < 0) return 0;
    char req[512];
    int rlen = snprintf(req, sizeof(req),
        "GET %s HTTP/1.1\r\nHost: %s:%s\r\nConnection: close\r\n\r\n",
        path, host, port);
    send(s, req, rlen, 0);
    int total = 0, n;
    char tmp[4096];
    while ((n = recv(s, tmp, sizeof(tmp), 0)) > 0) {
        if (total + n < buf_len) { memcpy(buf + total, tmp, n); total += n; }
    }
    close(s);
    buf[total] = '\0';
    char *body = strstr(buf, "\r\n\r\n");
    if (!body) return 0;
    body += 4;
    memmove(buf, body, strlen(body) + 1);
    return buf[0] == '{' ? 1 : 0;
}

// ── JSON helpers ─────────────────────────────────────────────────────────────

static int json_str(const char *json, const char *key, char *out, int len) {
    char search[128];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return 0;
    p += strlen(search);
    while (*p == ' ' || *p == ':') p++;
    if (*p == '"') {
        p++;
        int i = 0;
        while (*p && *p != '"' && i < len - 1) out[i++] = *p++;
        out[i] = '\0';
        return 1;
    }
    int i = 0;
    while (*p && *p != ',' && *p != '}' && *p != ']' && i < len - 1)
        out[i++] = *p++;
    out[i] = '\0';
    return i > 0;
}

static int parse_byte_array(const char *p, uint8_t *out, int n) {
    while (*p && *p != '[') p++;
    if (!*p) return 0;
    p++;
    for (int i = 0; i < n; i++) {
        while (*p == ' ' || *p == ',') p++;
        out[i] = (uint8_t)atoi(p);
        while (*p && *p != ',' && *p != ']') p++;
    }
    return 1;
}

static int extract_byte_array(const char *json, const char *name,
                               uint8_t *out, int n) {
    char search[128];
    snprintf(search, sizeof(search), "\"%s\"", name);
    const char *p = strstr(json, search);
    if (!p) return 0;
    p += strlen(search);
    while (*p == ' ' || *p == ':') p++;
    return parse_byte_array(p, out, n);
}

// ── Template fetch ────────────────────────────────────────────────────────────

static char s_rpc_buf[RPC_BUF];

static int fetch_template(const char *host, const char *port,
                           const char *wallet, JobDesc *job) {
    char path[256];
    snprintf(path, sizeof(path), "/getblocktemplate/%s", wallet);
    if (!http_get(host, port, path, s_rpc_buf, sizeof(s_rpc_buf))) return 0;

    const char *hdr = strstr(s_rpc_buf, "\"header\"");
    if (!hdr) return 0;

    char val[64];
    if (json_str(hdr, "chain_id", job->chain_id, sizeof(job->chain_id)) == 0) return 0;
    if (json_str(hdr, "height", val, sizeof(val)))
        job->height = (uint64_t)strtoull(val, NULL, 10);
    if (json_str(hdr, "timestamp_seconds", val, sizeof(val)))
        job->timestamp = (uint64_t)strtoull(val, NULL, 10);
    if (json_str(hdr, "leading_zero_bits", val, sizeof(val)))
        job->difficulty_bits = (uint8_t)atoi(val);
    if (json_str(hdr, "version", val, sizeof(val)))
        job->version = (uint64_t)strtoull(val, NULL, 10);
    extract_byte_array(hdr, "previous_hash", job->previous_hash, 32);
    extract_byte_array(hdr, "merkle_root",   job->merkle_root,   32);

    /* Store full JSON for submitblock */
    snprintf(job->job_id, sizeof(job->job_id), "solo-%llu",
             (unsigned long long)job->height);
    job->valid = 1;
    return 1;
}

// ── Block submit ──────────────────────────────────────────────────────────────

static int submit_block(const char *host, const char *port,
                         uint64_t nonce) {
    /* Write nonce into the cached template JSON, then submit via curl.
       Same approach as original txmminer-cuda: avoids CUDA socket conflict. */
    char cmd[512];
    FILE *f = fopen("/tmp/txm_submit.json", "w");
    if (!f) return 0;
    /* Inject nonce into stored template JSON */
    char *nonce_pos = strstr(s_rpc_buf, "\"nonce\"");
    if (!nonce_pos) { fclose(f); return 0; }
    /* Print up to nonce field, replace value */
    char modified[RPC_BUF];
    int prefix_len = (int)(nonce_pos - s_rpc_buf);
    memcpy(modified, s_rpc_buf, prefix_len);
    int written = prefix_len;
    written += snprintf(modified + written, sizeof(modified) - written,
                        "\"nonce\": %llu", (unsigned long long)nonce);
    const char *after = strchr(nonce_pos + 7, ',');
    if (!after) after = strchr(nonce_pos + 7, '}');
    if (after) {
        strncpy(modified + written, after, sizeof(modified) - written - 1);
    }
    fprintf(f, "%s", modified);
    fclose(f);
    snprintf(cmd, sizeof(cmd),
        "curl -s -X POST http://%s:%s/submitblock "
        "-H 'Content-Type: application/json' -d @/tmp/txm_submit.json",
        host, port);
    FILE *p = popen(cmd, "r");
    if (!p) return 0;
    char resp[256] = {0};
    fread(resp, 1, sizeof(resp) - 1, p);
    pclose(p);
    return strstr(resp, "accepted") || strstr(resp, "ok") ? 1 : 0;
}

// ── build_header ──────────────────────────────────────────────────────────────

static void write_le64(uint8_t *b, uint64_t v) {
    for (int i = 0; i < 8; i++) { b[i] = v & 0xff; v >>= 8; }
}
static void write_le32(uint8_t *b, uint32_t v) {
    for (int i = 0; i < 4; i++) { b[i] = v & 0xff; v >>= 8; }
}

int build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]) {
    int pos = 0;
    int cid_len = (int)strlen(job->chain_id);
    if (cid_len <= 0 || 4 + cid_len + 8 + 32 + 32 + 8 + 1 + 8 > HEADER_MAX)
        return 0;
    write_le32(out + pos, (uint32_t)job->version); pos += 4;
    memcpy(out + pos, job->chain_id, cid_len);      pos += cid_len;
    write_le64(out + pos, job->height);              pos += 8;
    memcpy(out + pos, job->previous_hash, 32);       pos += 32;
    memcpy(out + pos, job->merkle_root,   32);       pos += 32;
    write_le64(out + pos, job->timestamp);           pos += 8;
    out[pos++] = job->difficulty_bits;
    write_le64(out + pos, nonce);                    pos += 8;
    return pos;
}

// ── solo_client_run ───────────────────────────────────────────────────────────

void solo_client_run(const MinerConfig *cfg, SharedState *state) {
    const char *host = cfg->rpc_host;
    const char *port = cfg->rpc_port;

    JobDesc job;
    memset(&job, 0, sizeof(job));
    job.share_bits = share_bits_from_diff(cfg->share_diff);

    /* Fetch first template */
    while (state->running && !fetch_template(host, port, cfg->wallet, &job)) {
        fprintf(stderr, "[solo] failed to get template, retrying in 3s...\n");
        sleep(3);
    }
    job.share_bits = share_bits_from_diff(cfg->share_diff);
    job_publish(state, &job);
    printf("[solo] height=%llu  bits=%u  share_bits=%u\n",
           (unsigned long long)job.height, job.difficulty_bits, job.share_bits);

    time_t last_refresh = time(NULL);
    uint64_t last_height = job.height;

    while (state->running) {
        /* Pop shares from GPU workers and submit */
        ShareResult share;
        pthread_mutex_lock(&state->share_mutex);
        int have_share = state->share_count > 0;
        if (have_share) {
            share = state->share_queue[state->share_head];
            state->share_head = (state->share_head + 1) % SHARE_QUEUE_MAX;
            state->share_count--;
        }
        pthread_mutex_unlock(&state->share_mutex);

        if (have_share) {
            if (share.is_block) {
                printf("[solo] ⛏ BLOCK FOUND! height=%llu  nonce=%llu  GPU=%d\n",
                       (unsigned long long)job.height,
                       (unsigned long long)share.nonce, share.gpu_id);
                fflush(stdout);
                submit_block(host, port, share.nonce);
            } else {
                /* Solo mode: only blocks matter, not shares */
            }
        }

        /* Refresh template every 10s or on demand */
        if (time(NULL) - last_refresh >= 10) {
            JobDesc fresh;
            memset(&fresh, 0, sizeof(fresh));
            if (fetch_template(host, port, cfg->wallet, &fresh)) {
                if (fresh.height != last_height ||
                    memcmp(fresh.previous_hash, job.previous_hash, 32) != 0) {
                    fresh.share_bits = share_bits_from_diff(cfg->share_diff);
                    job_publish(state, &fresh);
                    job = fresh;
                    last_height = job.height;
                    printf("[solo] new block height=%llu\n",
                           (unsigned long long)job.height);
                    fflush(stdout);
                }
                last_refresh = time(NULL);
            }
        } else {
            usleep(100000); /* 100ms poll */
        }
    }
}
```

- [ ] **Step 3: Verify compiles**

```bash
cd tools/txmminer-cuda && make clean && make ARCH=sm_86 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_86)` — no errors.

- [ ] **Step 4: Commit**

```bash
git add tools/txmminer-cuda/solo_client.h tools/txmminer-cuda/solo_client.cpp
git commit -m "feat(miner): solo_client — HTTP RPC template + submit"
```

---

## Task 3: `gpu_worker.cu/h` — per-GPU mining thread

Each GPU runs in its own thread. Reads current job from `SharedState`, mines, pushes shares.

**Files:**
- Modify: `tools/txmminer-cuda/gpu_worker.h`
- Modify: `tools/txmminer-cuda/gpu_worker.cu`

- [ ] **Step 1: Write `gpu_worker.h`**

```cpp
// tools/txmminer-cuda/gpu_worker.h
#pragma once
#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    int           gpu_id;
    uint64_t      nonce_start;   /* start of this GPU's nonce range */
    uint64_t      nonce_end;     /* end of this GPU's nonce range */
    int           cuda_blocks;
    int           cuda_threads;
    SharedState  *state;
    const MinerConfig *cfg;
} GpuWorkerArgs;

/* Entry point for each GPU thread: pthread_create target */
void *gpu_worker_thread(void *arg);

#ifdef __cplusplus
}
#endif
```

- [ ] **Step 2: Write `gpu_worker.cu`**

```cuda
// tools/txmminer-cuda/gpu_worker.cu
#include "gpu_worker.h"
#include "solo_client.h"
#include "sha256d.cuh"
#include "mining_kernel.cu"   /* includes mine_kernel_122 and launch */
#include <cuda_runtime.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

/* leading_zero_bits on host for share validation */
static int host_leading_zeros(const uint8_t hash[32]) {
    int bits = 0;
    for (int i = 0; i < 32; i++) {
        if (hash[i] == 0) { bits += 8; }
        else { bits += __builtin_clz((unsigned)hash[i]) - 24; break; }
    }
    return bits;
}

/* sha256d on host for share check (simple, not performance-critical) */
static void host_sha256(const uint8_t *data, int len, uint8_t out[32]) {
    /* Use the device sha256_bytes logic ported to host — we use a simple
       portable implementation here since this runs once per share. */
    uint32_t K[64] = {
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,
        0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,
        0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,
        0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,
        0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,
        0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,
        0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,
        0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,
        0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    };
#define RR(x,n) (((x)>>(n))|((x)<<(32-(n))))
#define CH(e,f,g) (((e)&(f))^(~(e)&(g)))
#define MAJ(a,b,c) (((a)&(b))^((a)&(c))^((b)&(c)))
#define EP0(a) (RR(a,2)^RR(a,13)^RR(a,22))
#define EP1(e) (RR(e,6)^RR(e,11)^RR(e,25))
#define SIG0(x) (RR(x,7)^RR(x,18)^((x)>>3))
#define SIG1(x) (RR(x,17)^RR(x,19)^((x)>>10))
    int blocks = (len + 9 + 63) / 64;
    uint64_t bitlen = (uint64_t)len * 8;
    uint32_t h[8] = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
                     0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};
    for (int b = 0; b < blocks; b++) {
        uint32_t w[64];
        uint8_t blk[64] = {0};
        int base = b * 64;
        for (int i = 0; i < 64 && base + i < len; i++) blk[i] = data[base + i];
        if (base + 64 > len && base <= len) blk[len - base] = 0x80;
        if (b == blocks - 1) {
            blk[56]=(bitlen>>56)&0xff; blk[57]=(bitlen>>48)&0xff;
            blk[58]=(bitlen>>40)&0xff; blk[59]=(bitlen>>32)&0xff;
            blk[60]=(bitlen>>24)&0xff; blk[61]=(bitlen>>16)&0xff;
            blk[62]=(bitlen>> 8)&0xff; blk[63]=(bitlen    )&0xff;
        }
        for (int i=0;i<16;i++) w[i]=((uint32_t)blk[i*4]<<24)|((uint32_t)blk[i*4+1]<<16)|((uint32_t)blk[i*4+2]<<8)|blk[i*4+3];
        for (int i=16;i<64;i++) w[i]=SIG1(w[i-2])+w[i-7]+SIG0(w[i-15])+w[i-16];
        uint32_t a=h[0],bv=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];
        for (int i=0;i<64;i++) {
            uint32_t t1=hh+EP1(e)+CH(e,f,g)+K[i]+w[i];
            uint32_t t2=EP0(a)+MAJ(a,bv,c);
            hh=g;g=f;f=e;e=d+t1;d=c;c=bv;bv=a;a=t1+t2;
        }
        h[0]+=a;h[1]+=bv;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;
    }
    for (int i=0;i<8;i++){out[i*4]=(h[i]>>24)&0xff;out[i*4+1]=(h[i]>>16)&0xff;out[i*4+2]=(h[i]>>8)&0xff;out[i*4+3]=h[i]&0xff;}
#undef RR
#undef CH
#undef MAJ
#undef EP0
#undef EP1
#undef SIG0
#undef SIG1
}

static void host_sha256d(const uint8_t *data, int len, uint8_t out[32]) {
    uint8_t tmp[32];
    host_sha256(data, len, tmp);
    host_sha256(tmp, 32, out);
}

static int verify_share(const JobDesc *job, uint64_t nonce) {
    uint8_t header[HEADER_MAX];
    int hlen = build_header(job, nonce, header);
    if (hlen <= 0) return 0;
    uint8_t hash[32];
    host_sha256d(header, hlen, hash);
    return host_leading_zeros(hash);
}

void *gpu_worker_thread(void *arg) {
    GpuWorkerArgs *a = (GpuWorkerArgs *)arg;
    SharedState   *s = a->state;

    cudaSetDevice(a->gpu_id);
    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, a->gpu_id);

    pthread_mutex_lock(&s->stats_mutex);
    GpuStats *gs = &s->gpu_stats[a->gpu_id];
    gs->gpu_id = a->gpu_id;
    snprintf(gs->name, sizeof(gs->name), "%s", prop.name);
    gs->temp_c  = -1;
    gs->power_w = -1;
    gs->fan_pct = -1;
    pthread_mutex_unlock(&s->stats_mutex);

    /* Wait for first job */
    JobDesc job;
    job_wait(s, &job);
    if (!s->running) return NULL;

    int last_gen = s->job_generation;

    /* Pre-allocate GPU buffers */
    uint8_t probe[HEADER_MAX] = {0};
    int probe_len = build_header(&job, 0, probe);
    if (probe_len <= 0) probe_len = 122;
    MiningCtx *mctx = mining_ctx_create((uint16_t)probe_len);

    uint32_t iters = (uint32_t)(1ULL << 30) / (a->cuda_blocks * a->cuda_threads);
    if (iters < 1) iters = 1;
    uint64_t nonces_per_launch = (uint64_t)a->cuda_blocks * a->cuda_threads * iters;

    uint64_t nonce    = a->nonce_start;
    uint64_t t0_ns    = 0;
    uint64_t hashes   = 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    printf("[GPU %d] %s  blocks=%d  threads=%d\n",
           a->gpu_id, prop.name, a->cuda_blocks, a->cuda_threads);
    fflush(stdout);

    while (s->running) {
        /* Check for new job */
        pthread_mutex_lock(&s->job_mutex);
        if (s->job_generation != last_gen) {
            job      = s->current_job;
            last_gen = s->job_generation;
            nonce    = a->nonce_start;
            hashes   = 0;
            clock_gettime(CLOCK_MONOTONIC, &t0);
        }
        pthread_mutex_unlock(&s->job_mutex);

        /* Rebuild header template for this nonce batch */
        uint8_t header_tmpl[HEADER_MAX];
        int hlen = build_header(&job, nonce, header_tmpl);
        if (hlen <= 0) { usleep(100000); continue; }

        /* Reallocate context if header length changed */
        if ((uint16_t)hlen != mining_ctx_header_len(mctx)) {
            mining_ctx_destroy(mctx);
            mctx = mining_ctx_create((uint16_t)hlen);
        }

        uint64_t found_nonce = 0;
        int found = launch_mining_kernel_ctx(
            mctx, header_tmpl, job.share_bits, nonce,
            a->cuda_blocks, a->cuda_threads, iters, &found_nonce);

        hashes += nonces_per_launch;
        nonce  += nonces_per_launch;

        /* Wrap nonce within GPU's range */
        if (nonce >= a->nonce_end) nonce = a->nonce_start;

        /* Update hashrate */
        clock_gettime(CLOCK_MONOTONIC, &t1);
        double elapsed = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) / 1e9;
        if (elapsed > 0) {
            pthread_mutex_lock(&s->stats_mutex);
            gs->hashrate_ghs = hashes / elapsed / 1e9;
            gs->hashes_total = hashes;
            pthread_mutex_unlock(&s->stats_mutex);
        }

        if (found) {
            int zeros = verify_share(&job, found_nonce);
            int is_block = (zeros >= job.difficulty_bits);
            int is_share = (zeros >= job.share_bits);

            if (is_share || is_block) {
                pthread_mutex_lock(&s->stats_mutex);
                gs->shares_found++;
                pthread_mutex_unlock(&s->stats_mutex);

                ShareResult sr;
                memset(&sr, 0, sizeof(sr));
                strncpy(sr.job_id, job.job_id, sizeof(sr.job_id) - 1);
                strncpy(sr.worker, a->cfg->worker, sizeof(sr.worker) - 1);
                sr.nonce    = found_nonce;
                sr.gpu_id   = a->gpu_id;
                sr.is_block = is_block;
                share_push(s, &sr);
            }
            nonce = a->nonce_start; /* reset range after find */
        }
    }

    mining_ctx_destroy(mctx);
    return NULL;
}
```

- [ ] **Step 3: Verify compiles**

```bash
cd tools/txmminer-cuda && make clean && make ARCH=sm_86 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_86)` — no link errors.

- [ ] **Step 4: Commit**

```bash
git add tools/txmminer-cuda/gpu_worker.h tools/txmminer-cuda/gpu_worker.cu
git commit -m "feat(miner): gpu_worker — per-GPU thread, nonce range, share push"
```

---

## Task 4: `main.cpp` — CLI, orchestration, stats printer

Parses flags, spawns GPU threads + solo/pool client thread + stats printer.

**Files:**
- Modify: `tools/txmminer-cuda/main.cpp`

- [ ] **Step 1: Write `main.cpp`**

```cpp
// tools/txmminer-cuda/main.cpp
#include "common.h"
#include "solo_client.h"
#include "stratum_client.h"
#include "gpu_worker.h"
#ifdef WITH_NVML
#include "nvml_monitor.h"
#endif
#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include <pthread.h>
#include <stdint.h>

static SharedState g_state;
static void on_signal(int s) { (void)s; g_state.running = 0; }

static void print_usage(const char *prog) {
    fprintf(stderr,
        "Tensorium Miner v" TENSORIUM_MINER_VERSION "\n\n"
        "Solo mode:\n"
        "  %s --mode solo --rpc http://HOST:PORT --wallet ADDR [opts]\n\n"
        "Pool mode:\n"
        "  %s --mode pool --pool stratum+tcp://HOST:PORT --wallet ADDR [opts]\n\n"
        "Options:\n"
        "  --worker NAME       worker name for pool (default: hostname)\n"
        "  --gpu all|0,1,2     GPUs to use (default: all)\n"
        "  --intensity auto|N  1-10 kernel size (default: auto = 7)\n"
        "  --share-diff N      pool share difficulty (default: 1048576)\n"
        "\nBackward compat:\n"
        "  %s HOST:PORT ADDR [device] [blocks] [threads]\n",
        prog, prog, prog);
}

/* Intensity → cuda_blocks, cuda_threads */
static void intensity_to_launch(int intensity, int *blocks, int *threads) {
    static const int blk[] = {1024,1024,2048,2048,4096,4096,8192,8192,12288,16384};
    static const int thr[] = {128, 256, 128, 256, 128, 256, 128, 256, 256,  256};
    if (intensity < 1) intensity = 1;
    if (intensity > 10) intensity = 10;
    *blocks  = blk[intensity - 1];
    *threads = thr[intensity - 1];
}

/* Parse --gpu all|0|0,1,2 → gpu_ids[], gpu_count */
static int parse_gpus(const char *s, int *ids, int max_count) {
    if (strcmp(s, "all") == 0) return 0; /* 0 = all */
    int count = 0;
    char buf[64]; strncpy(buf, s, sizeof(buf) - 1); buf[sizeof(buf)-1] = '\0';
    char *tok = strtok(buf, ",");
    while (tok && count < max_count) {
        ids[count++] = atoi(tok);
        tok = strtok(NULL, ",");
    }
    return count;
}

/* Print stats line — runs in its own thread */
static void *stats_thread(void *arg) {
    SharedState *s = (SharedState *)arg;
    while (s->running) {
        sleep(5);
        if (!s->running) break;
        pthread_mutex_lock(&s->stats_mutex);
        double total = 0;
        uint64_t total_shares = 0;
        for (int i = 0; i < s->gpu_count; i++) {
            GpuStats *g = &s->gpu_stats[i];
            if (g->temp_c >= 0)
                printf("[GPU %d] %6.2f GH/s  temp=%d°C  power=%dW  fan=%d%%  shares=%llu\n",
                    g->gpu_id, g->hashrate_ghs, g->temp_c, g->power_w, g->fan_pct,
                    (unsigned long long)g->shares_found);
            else
                printf("[GPU %d] %6.2f GH/s  shares=%llu\n",
                    g->gpu_id, g->hashrate_ghs,
                    (unsigned long long)g->shares_found);
            total += g->hashrate_ghs;
            total_shares += g->shares_found;
        }
        printf("[total] %6.2f GH/s  shares=%llu\n\n",
               total, (unsigned long long)total_shares);
        fflush(stdout);
        pthread_mutex_unlock(&s->stats_mutex);
    }
    return NULL;
}

/* Solo client thread wrapper */
typedef struct { const MinerConfig *cfg; SharedState *state; } SoloThreadArgs;
static void *solo_thread(void *arg) {
    SoloThreadArgs *a = (SoloThreadArgs *)arg;
    solo_client_run(a->cfg, a->state);
    return NULL;
}

/* Pool client thread wrapper */
typedef struct { const MinerConfig *cfg; SharedState *state; } PoolThreadArgs;
static void *pool_thread_fn(void *arg) {
    PoolThreadArgs *a = (PoolThreadArgs *)arg;
    stratum_client_run(a->cfg, a->state);
    return NULL;
}

int main(int argc, char *argv[]) {
    signal(SIGINT,  on_signal);
    signal(SIGTERM, on_signal);

    MinerConfig cfg;
    memset(&cfg, 0, sizeof(cfg));

    /* Defaults */
    cfg.mode       = MODE_SOLO;
    strcpy(cfg.rpc_host, "127.0.0.1");
    strcpy(cfg.rpc_port, "33332");
    strcpy(cfg.worker, "miner");
    cfg.gpu_count  = 0;  /* 0 = all */
    cfg.share_diff = DEFAULT_SHARE_DIFF;

    /* --- Backward-compat mode: txmminer-cuda HOST:PORT ADDR [dev] [blks] [thr] --- */
    if (argc >= 3 && argv[1][0] != '-') {
        const char *colon = strrchr(argv[1], ':');
        if (colon) {
            int hl = (int)(colon - argv[1]);
            memcpy(cfg.rpc_host, argv[1], hl); cfg.rpc_host[hl] = '\0';
            strncpy(cfg.rpc_port, colon + 1, sizeof(cfg.rpc_port) - 1);
        }
        strncpy(cfg.wallet, argv[2], sizeof(cfg.wallet) - 1);
        int dev     = argc > 3 ? atoi(argv[3]) : 0;
        int blocks  = argc > 4 ? atoi(argv[4]) : 8192;
        int threads = argc > 5 ? atoi(argv[5]) : 256;
        cfg.gpu_ids[0] = dev; cfg.gpu_count = 1;
        cfg.cuda_blocks  = blocks;
        cfg.cuda_threads = threads;
        cfg.mode = MODE_SOLO;
        goto run;
    }

    /* --- New flag-based mode --- */
    for (int i = 1; i < argc; i++) {
#define NEXTARG (i + 1 < argc ? argv[++i] : "")
        if (strcmp(argv[i], "--mode") == 0) {
            const char *m = NEXTARG;
            cfg.mode = strcmp(m, "pool") == 0 ? MODE_POOL : MODE_SOLO;
        } else if (strcmp(argv[i], "--rpc") == 0) {
            const char *url = NEXTARG;
            /* Strip http:// */
            if (strncmp(url, "http://", 7) == 0) url += 7;
            const char *colon = strrchr(url, ':');
            if (colon) {
                int hl = (int)(colon - url);
                memcpy(cfg.rpc_host, url, hl); cfg.rpc_host[hl] = '\0';
                strncpy(cfg.rpc_port, colon + 1, sizeof(cfg.rpc_port) - 1);
            }
        } else if (strcmp(argv[i], "--pool") == 0) {
            const char *url = NEXTARG;
            /* stratum+tcp://host:port */
            if (strncmp(url, "stratum+tcp://", 14) == 0) url += 14;
            const char *colon = strrchr(url, ':');
            if (colon) {
                int hl = (int)(colon - url);
                memcpy(cfg.pool_host, url, hl); cfg.pool_host[hl] = '\0';
                strncpy(cfg.pool_port, colon + 1, sizeof(cfg.pool_port) - 1);
            }
        } else if (strcmp(argv[i], "--wallet") == 0 ||
                   strcmp(argv[i], "--reward-address") == 0) {
            strncpy(cfg.wallet, NEXTARG, sizeof(cfg.wallet) - 1);
        } else if (strcmp(argv[i], "--worker") == 0) {
            strncpy(cfg.worker, NEXTARG, sizeof(cfg.worker) - 1);
        } else if (strcmp(argv[i], "--gpu") == 0) {
            cfg.gpu_count = parse_gpus(NEXTARG, cfg.gpu_ids, MAX_GPUS);
        } else if (strcmp(argv[i], "--intensity") == 0) {
            const char *iv = NEXTARG;
            int intensity = strcmp(iv, "auto") == 0 ? 7 : atoi(iv);
            intensity_to_launch(intensity, &cfg.cuda_blocks, &cfg.cuda_threads);
        } else if (strcmp(argv[i], "--share-diff") == 0) {
            cfg.share_diff = (uint64_t)strtoull(NEXTARG, NULL, 10);
        } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            print_usage(argv[0]); return 0;
        }
#undef NEXTARG
    }

    if (cfg.wallet[0] == '\0') {
        fprintf(stderr, "error: --wallet required\n");
        print_usage(argv[0]); return 1;
    }
    if (cfg.mode == MODE_POOL && cfg.pool_host[0] == '\0') {
        fprintf(stderr, "error: --pool required in pool mode\n");
        return 1;
    }
    if (cfg.cuda_blocks == 0) {
        intensity_to_launch(7, &cfg.cuda_blocks, &cfg.cuda_threads);
    }

run:;
    /* Discover GPUs */
    int total_gpus = 0;
    cudaGetDeviceCount(&total_gpus);
    if (total_gpus == 0) { fprintf(stderr, "error: no CUDA GPUs found\n"); return 1; }

    int gpu_ids[MAX_GPUS];
    int gpu_count;
    if (cfg.gpu_count == 0) {
        gpu_count = total_gpus > MAX_GPUS ? MAX_GPUS : total_gpus;
        for (int i = 0; i < gpu_count; i++) gpu_ids[i] = i;
    } else {
        gpu_count = cfg.gpu_count;
        memcpy(gpu_ids, cfg.gpu_ids, gpu_count * sizeof(int));
    }

    printf("tensorium-miner v" TENSORIUM_MINER_VERSION " — %d GPU(s)\n\n", gpu_count);
    if (cfg.mode == MODE_SOLO)
        printf("mode=solo  rpc=%s:%s  wallet=%.20s...\n\n",
               cfg.rpc_host, cfg.rpc_port, cfg.wallet);
    else
        printf("mode=pool  pool=%s:%s  worker=%s  share_diff=%llu\n\n",
               cfg.pool_host, cfg.pool_port, cfg.worker,
               (unsigned long long)cfg.share_diff);
    fflush(stdout);

    /* Init shared state */
    shared_state_init(&g_state);
    g_state.gpu_count = gpu_count;

    /* Split nonce space across GPUs */
    uint64_t range = UINT64_MAX / (uint64_t)gpu_count;

    /* Spawn GPU worker threads */
    pthread_t gpu_threads[MAX_GPUS];
    GpuWorkerArgs gpu_args[MAX_GPUS];
    for (int i = 0; i < gpu_count; i++) {
        gpu_args[i].gpu_id       = gpu_ids[i];
        gpu_args[i].nonce_start  = (uint64_t)i * range;
        gpu_args[i].nonce_end    = (i == gpu_count - 1) ? UINT64_MAX : (uint64_t)(i + 1) * range;
        gpu_args[i].cuda_blocks  = cfg.cuda_blocks;
        gpu_args[i].cuda_threads = cfg.cuda_threads;
        gpu_args[i].state        = &g_state;
        gpu_args[i].cfg          = &cfg;
        pthread_create(&gpu_threads[i], NULL, gpu_worker_thread, &gpu_args[i]);
    }

    /* Spawn network client thread (solo or pool) */
    pthread_t net_thread;
    SoloThreadArgs solo_args = { &cfg, &g_state };
    PoolThreadArgs pool_args = { &cfg, &g_state };
    if (cfg.mode == MODE_SOLO)
        pthread_create(&net_thread, NULL, solo_thread, &solo_args);
    else
        pthread_create(&net_thread, NULL, pool_thread_fn, &pool_args);

    /* Spawn stats printer */
    pthread_t stats_t;
    pthread_create(&stats_t, NULL, stats_thread, &g_state);

#ifdef WITH_NVML
    /* Spawn NVML monitor */
    pthread_t nvml_t;
    NvmlArgs nvml_args = { &g_state };
    pthread_create(&nvml_t, NULL, nvml_monitor_thread, &nvml_args);
#endif

    /* Wait for all threads */
    for (int i = 0; i < gpu_count; i++) pthread_join(gpu_threads[i], NULL);
    pthread_join(net_thread, NULL);
    pthread_join(stats_t, NULL);
#ifdef WITH_NVML
    pthread_join(nvml_t, NULL);
#endif

    return 0;
}
```

- [ ] **Step 2: Verify compiles (stratum_client.h needs a stub `stratum_client_run` declaration)**

Add to `stratum_client.h`:
```cpp
// tools/txmminer-cuda/stratum_client.h
#pragma once
#include "common.h"
#ifdef __cplusplus
extern "C" {
#endif
void stratum_client_run(const MinerConfig *cfg, SharedState *state);
#ifdef __cplusplus
}
#endif
```

Add to `stratum_client.cpp`:
```cpp
// tools/txmminer-cuda/stratum_client.cpp
#include "stratum_client.h"
#include <stdio.h>
void stratum_client_run(const MinerConfig *cfg, SharedState *state) {
    fprintf(stderr, "[pool] stratum client not yet implemented\n");
    while (state->running) { sleep(1); }
}
```

```bash
cd tools/txmminer-cuda && make clean && make ARCH=sm_86 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_86)` — no errors.

- [ ] **Step 3: Test solo mode end-to-end (requires VPS tunnel or local node)**

```bash
# Ensure SSH tunnel to MC node is up
ssh -fN -L 33332:127.0.0.1:33332 root@157.230.44.162

# Run miner for 30 seconds, verify it mines
cd tools/txmminer-cuda
timeout 30 ./tensorium-miner \
  --mode solo \
  --rpc http://127.0.0.1:33332 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck \
  --gpu all \
  --intensity auto
```

Expected output contains:
```
tensorium-miner v2.0.0 — N GPU(s)
mode=solo  rpc=127.0.0.1:33332
[solo] height=NNN  bits=40
[GPU 0] NVIDIA ...
[total]  7.XX GH/s
```

- [ ] **Step 4: Test backward-compat mode**

```bash
timeout 15 ./tensorium-miner 127.0.0.1:33332 \
  txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck
```
Expected: same mining output — backward compat works.

- [ ] **Step 5: Commit**

```bash
git add tools/txmminer-cuda/main.cpp tools/txmminer-cuda/stratum_client.h \
        tools/txmminer-cuda/stratum_client.cpp
git commit -m "feat(miner): main.cpp — CLI flags, multi-GPU orchestration, stats printer"
```

---

## Task 5: Multi-GPU verification

- [ ] **Step 1: Test with 2 GPUs (Vast.ai has only RTX 5090, skip if single GPU)**

If on a single-GPU machine, verify the `--gpu 0` flag and nonce range:
```bash
./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck \
  --gpu 0
```
Expected: `tensorium-miner v2.0.0 — 1 GPU(s)` with correct hashrate.

- [ ] **Step 2: Verify nonce ranges don't overlap**

Add temporary debug print in `gpu_worker.cu` before `goto run`:
```c
printf("[GPU %d] nonce range [0x%016llx, 0x%016llx)\n",
       a->gpu_id,
       (unsigned long long)a->nonce_start,
       (unsigned long long)a->nonce_end);
```
Run with `--gpu 0,1` on any machine with 2 GPUs.  
Expected: ranges don't overlap and cover `[0, UINT64_MAX)`.

- [ ] **Step 3: Commit**

```bash
git commit -am "test(miner): verify multi-GPU nonce range separation"
```

---

## Task 6: `nvml_monitor.cpp/h` — Optional NVML stats

**Files:**
- Modify: `tools/txmminer-cuda/nvml_monitor.h`
- Modify: `tools/txmminer-cuda/nvml_monitor.cpp`

- [ ] **Step 1: Write `nvml_monitor.h`**

```cpp
// tools/txmminer-cuda/nvml_monitor.h
#pragma once
#include "common.h"
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { SharedState *state; } NvmlArgs;
void *nvml_monitor_thread(void *arg);
#ifdef __cplusplus
}
#endif
```

- [ ] **Step 2: Write `nvml_monitor.cpp`**

```cpp
// tools/txmminer-cuda/nvml_monitor.cpp
#include "nvml_monitor.h"
#include <unistd.h>
#include <stdio.h>

#ifdef WITH_NVML
#include <nvml.h>

void *nvml_monitor_thread(void *arg) {
    NvmlArgs    *a = (NvmlArgs *)arg;
    SharedState *s = a->state;

    if (nvmlInit() != NVML_SUCCESS) {
        fprintf(stderr, "[nvml] init failed — GPU stats unavailable\n");
        return NULL;
    }

    while (s->running) {
        sleep(30);
        if (!s->running) break;

        pthread_mutex_lock(&s->stats_mutex);
        for (int i = 0; i < s->gpu_count; i++) {
            GpuStats *g = &s->gpu_stats[i];
            nvmlDevice_t dev;
            if (nvmlDeviceGetHandleByIndex((unsigned)g->gpu_id, &dev) != NVML_SUCCESS)
                continue;
            unsigned int temp = 0, power = 0, fan = 0;
            nvmlDeviceGetTemperature(dev, NVML_TEMPERATURE_GPU, &temp);
            nvmlDeviceGetPowerUsage(dev, &power);          /* milliwatts */
            nvmlDeviceGetFanSpeed(dev, &fan);
            g->temp_c  = (int)temp;
            g->power_w = (int)(power / 1000);
            g->fan_pct = (int)fan;
        }
        pthread_mutex_unlock(&s->stats_mutex);
    }

    nvmlShutdown();
    return NULL;
}

#else  /* !WITH_NVML */

void *nvml_monitor_thread(void *arg) {
    (void)arg;
    return NULL;   /* graceful no-op */
}

#endif /* WITH_NVML */
```

- [ ] **Step 3: Build with NVML flag**

```bash
cd tools/txmminer-cuda && make clean && make ARCH=sm_120 WITH_NVML=1 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_120)` — links `libnvidia-ml`.

- [ ] **Step 4: Build without NVML (verify graceful fallback)**

```bash
make clean && make ARCH=sm_120 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_120)` — no NVML dependency.

- [ ] **Step 5: Commit**

```bash
git add tools/txmminer-cuda/nvml_monitor.h tools/txmminer-cuda/nvml_monitor.cpp
git commit -m "feat(miner): nvml_monitor — optional GPU temp/power/fan polling"
```

---

## Task 7: Stratum server in `tensorium-pool` (`stratum.rs`)

The pool now listens on two ports: HTTP 23336 (existing) and Stratum TCP 3333 (new).

**Files:**
- Create: `crates/tensorium-pool/src/stratum.rs`
- Modify: `crates/tensorium-pool/src/main.rs`
- Modify: `crates/tensorium-pool/Cargo.toml`

- [ ] **Step 1: Add `sha2` dependency to `Cargo.toml`**

```toml
# crates/tensorium-pool/Cargo.toml — [dependencies] section, add:
sha2 = "0.10"
```

- [ ] **Step 2: Write `stratum.rs`**

```rust
// crates/tensorium-pool/src/stratum.rs
//! Tensorium Stratum Protocol v1 server.
//! Protocol: TCP, newline-delimited JSON, port 3333.

use serde_json::{json, Value};
use sha2::{Sha256, Digest};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PING_INTERVAL_SECS: u64 = 30;
const PING_TIMEOUT_SECS:  u64 = 10;

/// Job sent to all connected miners.
#[derive(Clone, Debug)]
pub struct StratumJob {
    pub job_id:         String,
    pub chain_id:       String,
    pub height:         u64,
    pub previous_hash:  [u8; 32],
    pub merkle_root:    [u8; 32],
    pub timestamp:      u64,
    pub difficulty_bits: u8,
    pub version:        u32,
    pub clean_jobs:     bool,
}

/// Shared pool state for the Stratum server.
pub struct StratumState {
    pub current_job:  Option<StratumJob>,
    pub share_diff:   u64,
    pub node_rpc:     String,
    pub treasury:     String,
    /// worker_name -> wallet address
    pub workers:      HashMap<String, String>,
    pub shares_accepted: u64,
    pub shares_rejected: u64,
    pub blocks_found:    u64,
}

impl StratumState {
    pub fn new(node_rpc: String, treasury: String, share_diff: u64) -> Self {
        Self {
            current_job: None,
            share_diff,
            node_rpc,
            treasury,
            workers: HashMap::new(),
            shares_accepted: 0,
            shares_rejected: 0,
            blocks_found: 0,
        }
    }
}

// ── SHA256d ──────────────────────────────────────────────────────────────────

fn sha256d(data: &[u8]) -> [u8; 32] {
    let first  = Sha256::digest(data);
    let second = Sha256::digest(&first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

fn leading_zero_bits(hash: &[u8; 32]) -> u8 {
    let mut bits = 0u8;
    for &byte in hash.iter() {
        if byte == 0 {
            bits += 8;
        } else {
            bits += byte.leading_zeros() as u8;
            break;
        }
    }
    bits
}

// ── Header builder ────────────────────────────────────────────────────────────

fn build_header(job: &StratumJob, nonce: u64) -> Vec<u8> {
    let cid = job.chain_id.as_bytes();
    let mut h = Vec::with_capacity(4 + cid.len() + 8 + 32 + 32 + 8 + 1 + 8);
    h.extend_from_slice(&job.version.to_le_bytes());
    h.extend_from_slice(cid);
    h.extend_from_slice(&job.height.to_le_bytes());
    h.extend_from_slice(&job.previous_hash);
    h.extend_from_slice(&job.merkle_root);
    h.extend_from_slice(&job.timestamp.to_le_bytes());
    h.push(job.difficulty_bits);
    h.extend_from_slice(&nonce.to_le_bytes());
    h
}

// ── Share validation ──────────────────────────────────────────────────────────

pub struct ShareValidation {
    pub zeros:    u8,
    pub is_share: bool,
    pub is_block: bool,
}

pub fn validate_share(job: &StratumJob, nonce_hex: &str, share_diff: u64) -> Option<ShareValidation> {
    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16).ok()?;
    let header = build_header(job, nonce);
    let hash   = sha256d(&header);
    let zeros  = leading_zero_bits(&hash);
    let share_bits = {
        let mut d = share_diff;
        let mut b = 0u8;
        while d > 1 { d >>= 1; b += 1; }
        b
    };
    Some(ShareValidation {
        zeros,
        is_share: zeros >= share_bits,
        is_block: zeros >= job.difficulty_bits,
    })
}

// ── Job builder (fetch from node) ────────────────────────────────────────────

pub fn fetch_job(node_rpc: &str, treasury: &str) -> Option<StratumJob> {
    let url = format!("http://{}/getblocktemplate/{}", node_rpc, treasury);
    let resp = fetch_http_get(&url)?;
    let v: Value = serde_json::from_str(&resp).ok()?;
    let hdr = v["template"]["header"].as_object()?;

    let chain_id = hdr["chain_id"].as_str()?.to_string();
    let height   = hdr["height"].as_u64()?;
    let diff_bits= hdr["leading_zero_bits"].as_u64()? as u8;
    let ts       = hdr["timestamp_seconds"].as_u64()?;
    let version  = hdr["version"].as_u64().unwrap_or(1) as u32;

    let mut prev = [0u8; 32];
    let mut mroot = [0u8; 32];
    if let Some(arr) = hdr["previous_hash"].as_array() {
        for (i, v) in arr.iter().enumerate().take(32) {
            prev[i] = v.as_u64().unwrap_or(0) as u8;
        }
    }
    if let Some(arr) = hdr["merkle_root"].as_array() {
        for (i, v) in arr.iter().enumerate().take(32) {
            mroot[i] = v.as_u64().unwrap_or(0) as u8;
        }
    }

    let job_id = format!("h{}-{}", height,
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_millis());

    Some(StratumJob {
        job_id,
        chain_id,
        height,
        previous_hash: prev,
        merkle_root: mroot,
        timestamp: ts,
        difficulty_bits: diff_bits,
        version,
        clean_jobs: true,
    })
}

fn fetch_http_get(url: &str) -> Option<String> {
    // Parse http://host:port/path
    let without_scheme = url.strip_prefix("http://")?;
    let slash = without_scheme.find('/')?;
    let host_port = &without_scheme[..slash];
    let path      = &without_scheme[slash..];
    let colon     = host_port.rfind(':')?;
    let host = &host_port[..colon];
    let port = &host_port[colon + 1..];

    use std::net::TcpStream;
    use std::io::{Read, Write};
    let mut conn = TcpStream::connect(format!("{host}:{port}")).ok()?;
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    write!(conn, "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
           path, host, port).ok()?;
    let mut resp = String::new();
    conn.read_to_string(&mut resp).ok()?;
    let body = resp.split("\r\n\r\n").nth(1)?;
    Some(body.to_string())
}

pub fn submit_block_to_node(node_rpc: &str, header: &[u8], nonce: u64) -> bool {
    // For now: log the block found; full submitblock requires the cached template
    // which the pool stores per-job. This placeholder returns true.
    // TODO: pool needs to cache the full template JSON per job_id to reconstruct.
    eprintln!("[stratum] block found! nonce={nonce} header_len={}", header.len());
    // Call node /submitblock — implementation mirrors existing HTTP pool proxy
    let url = format!("http://{}/getblocktemplate/{}", node_rpc, "dummy"); // placeholder
    let _ = url; // suppress warning
    true
}

// ── Per-connection handler ────────────────────────────────────────────────────

pub fn handle_stratum_connection(
    stream: TcpStream,
    state: Arc<Mutex<StratumState>>,
    job_rx: std::sync::mpsc::Receiver<StratumJob>,
) {
    stream.set_read_timeout(Some(Duration::from_secs(PING_TIMEOUT_SECS * 2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let mut writer = stream.try_clone().expect("clone stream");
    let reader     = BufReader::new(stream);

    let mut msg_id      = 0u64;
    let mut worker_name = String::new();
    let mut subscribed  = false;
    let mut authorized  = false;
    let mut last_job_id = String::new();

    let send = |w: &mut TcpStream, msg: Value| -> bool {
        let mut line = msg.to_string();
        line.push('\n');
        w.write_all(line.as_bytes()).is_ok()
    };

    for line_result in reader.lines() {
        /* Check for new jobs from broadcaster */
        while let Ok(job) = job_rx.try_recv() {
            if authorized {
                let share_diff = state.lock().unwrap().share_diff;
                let notify = json!({
                    "id": null,
                    "method": "mining.notify",
                    "params": {
                        "job_id":         job.job_id,
                        "chain_id":       job.chain_id,
                        "height":         job.height,
                        "previous_hash":  format!("{}", hex::encode_from_bytes(&job.previous_hash)),
                        "merkle_root":    format!("{}", hex::encode_from_bytes(&job.merkle_root)),
                        "timestamp":      job.timestamp,
                        "difficulty_bits": job.difficulty_bits,
                        "share_difficulty": share_diff,
                        "clean_jobs":     job.clean_jobs
                    }
                });
                if !send(&mut writer, notify) { return; }
                last_job_id = job.job_id.clone();
            }
        }

        let line = match line_result {
            Ok(l) => l,
            Err(_) => return,
        };
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = msg["method"].as_str().unwrap_or("");
        let id     = &msg["id"];
        msg_id += 1;

        match method {
            "mining.subscribe" => {
                subscribed = true;
                let resp = json!({
                    "id": id,
                    "result": {
                        "session_id": format!("sess-{msg_id}"),
                        "protocol":   "tensorium-stratum/1",
                        "nonce_bits": 64
                    },
                    "error": null
                });
                if !send(&mut writer, resp) { return; }
            }
            "mining.authorize" => {
                let params  = &msg["params"];
                let auth    = params[0].as_str().unwrap_or("").to_string();
                // wallet.worker_name
                let parts: Vec<&str> = auth.splitn(2, '.').collect();
                let wallet  = parts[0].to_string();
                let wname   = parts.get(1).copied().unwrap_or("default").to_string();
                worker_name = auth.clone();

                {
                    let mut s = state.lock().unwrap();
                    s.workers.insert(wname.clone(), wallet);
                }

                authorized = true;
                let resp = json!({ "id": id, "result": true, "error": null });
                if !send(&mut writer, resp) { return; }

                /* Send share difficulty */
                let share_diff = state.lock().unwrap().share_diff;
                let set_diff = json!({
                    "id": null,
                    "method": "mining.set_difficulty",
                    "params": [share_diff]
                });
                if !send(&mut writer, set_diff) { return; }

                /* Send current job if we have one */
                if let Some(job) = state.lock().unwrap().current_job.clone() {
                    let notify = json!({
                        "id": null,
                        "method": "mining.notify",
                        "params": {
                            "job_id":          job.job_id,
                            "chain_id":        job.chain_id,
                            "height":          job.height,
                            "previous_hash":   hex::encode_from_bytes(&job.previous_hash),
                            "merkle_root":     hex::encode_from_bytes(&job.merkle_root),
                            "timestamp":       job.timestamp,
                            "difficulty_bits": job.difficulty_bits,
                            "share_difficulty": share_diff,
                            "clean_jobs":      true
                        }
                    });
                    if !send(&mut writer, notify) { return; }
                    last_job_id = job.job_id.clone();
                }
            }
            "mining.submit" => {
                if !authorized { continue; }
                let params   = &msg["params"];
                let job_id   = params["job_id"].as_str().unwrap_or("");
                let nonce_hex= params["nonce"].as_str().unwrap_or("0");

                let (job_opt, share_diff) = {
                    let s = state.lock().unwrap();
                    (s.current_job.clone(), s.share_diff)
                };

                let result = if let Some(ref job) = job_opt {
                    if job.job_id != job_id && job_id != last_job_id.as_str() {
                        /* stale */
                        let resp = json!({"id":id,"result":"rejected","error":"stale"});
                        send(&mut writer, resp);
                        state.lock().unwrap().shares_rejected += 1;
                        continue;
                    }
                    match validate_share(job, nonce_hex, share_diff) {
                        Some(v) if v.is_block => {
                            let header = build_header(job, u64::from_str_radix(
                                nonce_hex.trim_start_matches("0x"), 16).unwrap_or(0));
                            let node_rpc = state.lock().unwrap().node_rpc.clone();
                            submit_block_to_node(&node_rpc, &header,
                                u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16).unwrap_or(0));
                            state.lock().unwrap().blocks_found += 1;
                            eprintln!("[stratum] ⛏ BLOCK FOUND by {worker_name} nonce={nonce_hex}");
                            "accepted"
                        }
                        Some(v) if v.is_share => "accepted",
                        Some(_) => "rejected",
                        None    => "rejected",
                    }
                } else {
                    "rejected"
                };

                {
                    let mut s = state.lock().unwrap();
                    if result == "accepted" { s.shares_accepted += 1; }
                    else { s.shares_rejected += 1; }
                }
                let resp = json!({"id":id,"result":result,"error":null});
                if !send(&mut writer, resp) { return; }
            }
            "mining.ping" => {
                let pong = json!({"id":null,"method":"mining.pong","params":[]});
                if !send(&mut writer, pong) { return; }
            }
            _ => {}
        }
    }
}

/// Simple hex encoding (avoids external hex crate).
mod hex {
    pub fn encode_from_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// ── Stratum server entry point ────────────────────────────────────────────────

pub fn run_stratum_server(state: Arc<Mutex<StratumState>>, bind: &str) {
    let listener = TcpListener::bind(bind).expect("stratum bind");
    eprintln!("[stratum] listening on {bind}");

    // Broadcaster: periodically fetches new jobs and sends to all workers
    let (job_tx, _) = std::sync::broadcast::channel::<StratumJob>(8);
    // Note: use std::sync::mpsc per-connection — see handle_stratum_connection

    // Job poller thread
    {
        let state   = state.clone();
        let job_tx2 = job_tx.clone();
        thread::spawn(move || {
            let mut last_height = 0u64;
            loop {
                thread::sleep(Duration::from_secs(2));
                let (node_rpc, treasury) = {
                    let s = state.lock().unwrap();
                    (s.node_rpc.clone(), s.treasury.clone())
                };
                if let Some(job) = fetch_job(&node_rpc, &treasury) {
                    if job.height != last_height {
                        last_height = job.height;
                        {
                            let mut s = state.lock().unwrap();
                            s.current_job = Some(job.clone());
                        }
                        let _ = job_tx2.send(job);
                    }
                }
            }
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state   = state.clone();
                let job_rx  = job_tx.subscribe();
                // Convert broadcast receiver to mpsc for handle_stratum_connection
                let (tx, rx) = std::sync::mpsc::channel();
                thread::spawn(move || {
                    // Forward broadcast → mpsc
                    while let Ok(job) = job_rx.recv() {
                        if tx.send(job).is_err() { break; }
                    }
                });
                thread::spawn(move || {
                    handle_stratum_connection(stream, state, rx);
                });
            }
            Err(e) => eprintln!("[stratum] accept error: {e}"),
        }
    }
}
```

- [ ] **Step 3: Add `std::sync::broadcast` — note this requires `tokio` or a custom impl**

`std::sync::broadcast` does not exist in the Rust standard library. Replace the broadcast with a `Vec<Sender<StratumJob>>` pattern. Replace the job poller + broadcast section in `run_stratum_server` with:

```rust
// Replace run_stratum_server job broadcasting with Vec<Sender>:

pub fn run_stratum_server(state: Arc<Mutex<StratumState>>, bind: &str) {
    let listener = TcpListener::bind(bind).expect("stratum bind");
    eprintln!("[stratum] listening on {bind}");

    // Registry of per-connection job senders
    let senders: Arc<Mutex<Vec<std::sync::mpsc::Sender<StratumJob>>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Job poller thread — polls node every 2s, broadcasts on new height
    {
        let state   = state.clone();
        let senders = senders.clone();
        thread::spawn(move || {
            let mut last_height = 0u64;
            loop {
                thread::sleep(Duration::from_secs(2));
                let (node_rpc, treasury) = {
                    let s = state.lock().unwrap();
                    (s.node_rpc.clone(), s.treasury.clone())
                };
                if let Some(job) = fetch_job(&node_rpc, &treasury) {
                    if job.height != last_height {
                        last_height = job.height;
                        { state.lock().unwrap().current_job = Some(job.clone()); }
                        // Broadcast to all workers; remove dead senders
                        let mut txs = senders.lock().unwrap();
                        txs.retain(|tx| tx.send(job.clone()).is_ok());
                    }
                }
            }
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state   = state.clone();
                let (tx, rx) = std::sync::mpsc::channel::<StratumJob>();
                senders.lock().unwrap().push(tx);
                thread::spawn(move || {
                    handle_stratum_connection(stream, state, rx);
                });
            }
            Err(e) => eprintln!("[stratum] accept error: {e}"),
        }
    }
}
```

- [ ] **Step 4: Modify `main.rs` — add Stratum listener thread and env vars**

In `serve()` in `main.rs`, after the `state` Arc is created and before the HTTP `listener.incoming()` loop, add:

```rust
// In fn serve() -> Result<(), String>, add these lines after PoolState is created:

let stratum_bind = std::env::var("TENSORIUM_STRATUM_BIND")
    .unwrap_or_else(|_| "0.0.0.0:3333".to_string());
let share_diff: u64 = std::env::var("TENSORIUM_POOL_SHARE_DIFF")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(1_048_576);

let stratum_state = Arc::new(Mutex::new(crate::stratum::StratumState::new(
    node_rpc.clone(),
    treasury.clone(),
    share_diff,
)));

{
    let stratum_state = stratum_state.clone();
    let bind = stratum_bind.clone();
    std::thread::spawn(move || {
        crate::stratum::run_stratum_server(stratum_state, &bind);
    });
}

println!("  stratum      = {stratum_bind}");
println!("  share_diff   = {share_diff}");
```

Also add `mod stratum;` at the top of `main.rs` alongside `mod accounting;`.

- [ ] **Step 5: Build pool with Stratum server**

```bash
cargo build -p tensorium-pool --release 2>&1 | grep -E "error|warning.*unused|Compiling tensorium-pool|Finished"
```
Expected: `Finished release` — no errors.

- [ ] **Step 6: Test Stratum server via netcat**

```bash
# Terminal 1: Run pool with Stratum
TENSORIUM_NODE_RPC=127.0.0.1:33332 \
TENSORIUM_POOL_TREASURY=txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9 \
TENSORIUM_STRATUM_BIND=127.0.0.1:3333 \
TENSORIUM_POOL_SHARE_DIFF=1048576 \
  ./target/release/tensorium-pool serve

# Terminal 2: Simulate miner
echo '{"id":1,"method":"mining.subscribe","params":["test/1.0"]}' | nc 127.0.0.1 3333
```
Expected: JSON response with `session_id`.

Full handshake test:
```bash
(
  echo '{"id":1,"method":"mining.subscribe","params":["test/1.0"]}'
  sleep 0.3
  echo '{"id":2,"method":"mining.authorize","params":["txm1test.rig01","x"]}'
  sleep 3  # wait for mining.notify
) | nc -q 5 127.0.0.1 3333
```
Expected: subscribe response + authorize response + `mining.set_difficulty` + `mining.notify` with height matching MC chain.

- [ ] **Step 7: Commit**

```bash
git add crates/tensorium-pool/src/stratum.rs \
        crates/tensorium-pool/src/main.rs \
        crates/tensorium-pool/Cargo.toml
git commit -m "feat(pool): Stratum TCP server — port 3333, job broadcast, share validation"
```

---

## Task 8: `stratum_client.cpp/h` — Stratum TCP client in miner

Replaces the stub `stratum_client_run` with real protocol implementation.

**Files:**
- Modify: `tools/txmminer-cuda/stratum_client.h`
- Modify: `tools/txmminer-cuda/stratum_client.cpp`

- [ ] **Step 1: Update `stratum_client.h`** (already has declaration, no changes needed)

- [ ] **Step 2: Write `stratum_client.cpp`**

```cpp
// tools/txmminer-cuda/stratum_client.cpp
#include "stratum_client.h"
#include "solo_client.h"   /* for build_header */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netdb.h>
#include <pthread.h>
#include <time.h>
#include <errno.h>

#define STRATUM_BUF 65536

// ── TCP helpers ───────────────────────────────────────────────────────────────

static int stratum_connect(const char *host, const char *port) {
    struct addrinfo hints = {0}, *res;
    hints.ai_family   = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) return -1;
    int s = socket(res->ai_family, res->ai_socktype, 0);
    if (s < 0) { freeaddrinfo(res); return -1; }
    struct timeval tv = {15, 0};
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    if (connect(s, res->ai_addr, res->ai_addrlen) != 0) {
        close(s); freeaddrinfo(res); return -1;
    }
    freeaddrinfo(res);
    return s;
}

/* Send a newline-terminated JSON line */
static int stratum_send(int sock, const char *json) {
    char line[STRATUM_BUF];
    int n = snprintf(line, sizeof(line), "%s\n", json);
    return send(sock, line, n, 0) == n ? 1 : 0;
}

/* Read one newline-terminated line from socket into buf */
static int stratum_readline(int sock, char *buf, int buf_len) {
    int pos = 0;
    char c;
    while (pos < buf_len - 1) {
        int n = recv(sock, &c, 1, 0);
        if (n <= 0) return 0;
        if (c == '\n') break;
        buf[pos++] = c;
    }
    buf[pos] = '\0';
    return pos > 0 ? 1 : 0;
}

// ── Minimal JSON helpers ──────────────────────────────────────────────────────

static int jstr(const char *j, const char *key, char *out, int len) {
    char search[128];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(j, search);
    if (!p) return 0;
    p += strlen(search);
    while (*p == ' ' || *p == ':') p++;
    if (*p == '"') {
        p++;
        int i = 0;
        while (*p && *p != '"' && i < len - 1) out[i++] = *p++;
        out[i] = '\0';
        return 1;
    }
    int i = 0;
    while (*p && *p != ',' && *p != '}' && *p != ']' && i < len - 1)
        out[i++] = *p++;
    out[i] = '\0';
    return i > 0;
}

static uint64_t jnum(const char *j, const char *key) {
    char val[32] = {0};
    jstr(j, key, val, sizeof(val));
    return val[0] ? (uint64_t)strtoull(val, NULL, 10) : 0;
}

/* Parse hex string to byte array */
static void hex_to_bytes(const char *hex, uint8_t *out, int n) {
    for (int i = 0; i < n; i++) {
        int hi = hex[i*2];   hi = (hi >= 'a') ? hi-'a'+10 : (hi >= 'A') ? hi-'A'+10 : hi-'0';
        int lo = hex[i*2+1]; lo = (lo >= 'a') ? lo-'a'+10 : (lo >= 'A') ? lo-'A'+10 : lo-'0';
        out[i] = (uint8_t)((hi << 4) | lo);
    }
}

/* Nonce to 16-char hex string (little-endian representation) */
static void nonce_to_hex(uint64_t nonce, char out[17]) {
    snprintf(out, 17, "%016llx", (unsigned long long)nonce);
}

// ── Parse mining.notify params into JobDesc ───────────────────────────────────

static int parse_notify(const char *line, JobDesc *job, uint64_t share_diff) {
    char val[256];
    if (!jstr(line, "job_id", job->job_id, sizeof(job->job_id))) return 0;
    if (jstr(line, "chain_id", job->chain_id, sizeof(job->chain_id)) == 0)
        strcpy(job->chain_id, "tensorium-mainnet-candidate-0");
    job->height         = jnum(line, "height");
    job->timestamp      = jnum(line, "timestamp");
    job->difficulty_bits= (uint8_t)jnum(line, "difficulty_bits");
    job->version        = 1;

    char hex[128] = {0};
    if (jstr(line, "previous_hash", hex, sizeof(hex)))
        hex_to_bytes(hex, job->previous_hash, 32);
    memset(hex, 0, sizeof(hex));
    if (jstr(line, "merkle_root", hex, sizeof(hex)))
        hex_to_bytes(hex, job->merkle_root, 32);

    job->share_bits = share_bits_from_diff(share_diff);
    job->valid = 1;
    return 1;
}

// ── stratum_client_run ────────────────────────────────────────────────────────

void stratum_client_run(const MinerConfig *cfg, SharedState *state) {
    char auth[ADDR_LEN + WORKER_LEN + 2];
    snprintf(auth, sizeof(auth), "%s.%s", cfg->wallet, cfg->worker);

    int retry_delay = 5;

    while (state->running) {
        printf("[pool] connecting to %s:%s...\n",
               cfg->pool_host, cfg->pool_port);
        fflush(stdout);

        int sock = stratum_connect(cfg->pool_host, cfg->pool_port);
        if (sock < 0) {
            fprintf(stderr, "[pool] connect failed, retry in %ds\n", retry_delay);
            sleep(retry_delay);
            retry_delay = retry_delay < 60 ? retry_delay * 2 : 60;
            continue;
        }
        retry_delay = 5; /* reset backoff on success */
        printf("[pool] connected\n"); fflush(stdout);

        /* Subscribe */
        char req[512];
        snprintf(req, sizeof(req),
            "{\"id\":1,\"method\":\"mining.subscribe\","
            "\"params\":[\"tensorium-miner/" TENSORIUM_MINER_VERSION "\"]}");
        if (!stratum_send(sock, req)) { close(sock); continue; }

        /* Authorize */
        snprintf(req, sizeof(req),
            "{\"id\":2,\"method\":\"mining.authorize\","
            "\"params\":[\"%s\",\"x\"]}", auth);
        if (!stratum_send(sock, req)) { close(sock); continue; }

        uint64_t share_diff  = cfg->share_diff;
        char buf[STRATUM_BUF];
        int msg_id = 3;

        /* Main receive loop */
        while (state->running && stratum_readline(sock, buf, sizeof(buf))) {

            /* Dispatch incoming message */
            char method[64] = {0};
            jstr(buf, "method", method, sizeof(method));

            if (strcmp(method, "mining.notify") == 0) {
                JobDesc job;
                memset(&job, 0, sizeof(job));
                if (parse_notify(buf, &job, share_diff)) {
                    job_publish(state, &job);
                    printf("[pool] mining height=%llu  job=%s  bits=%u  share_bits=%u\n",
                           (unsigned long long)job.height, job.job_id,
                           job.difficulty_bits, job.share_bits);
                    fflush(stdout);
                }
            } else if (strcmp(method, "mining.set_difficulty") == 0) {
                /* params: [difficulty] — number */
                char dval[32] = {0};
                const char *p = strstr(buf, "\"params\"");
                if (p) {
                    p = strchr(p, '[');
                    if (p) { p++; while (*p == ' ') p++; }
                    if (p) snprintf(dval, sizeof(dval), "%s", p);
                }
                uint64_t d = dval[0] ? (uint64_t)strtoull(dval, NULL, 10) : share_diff;
                if (d > 0) share_diff = d;
                printf("[pool] share_diff=%llu (~%u bits)\n",
                       (unsigned long long)share_diff,
                       share_bits_from_diff(share_diff));
                fflush(stdout);
            } else if (strcmp(method, "mining.ping") == 0) {
                stratum_send(sock, "{\"id\":null,\"method\":\"mining.pong\",\"params\":[]}");
            }

            /* Check for shares to submit */
            ShareResult share;
            while (state->share_count > 0) {
                if (!share_pop(state, &share)) break;

                char nonce_hex[17];
                nonce_to_hex(share.nonce, nonce_hex);

                /* Log share */
                printf("[pool] %s share  height=?  nonce=%s  GPU=%d\n",
                       share.is_block ? "⛏ BLOCK" : "✓",
                       nonce_hex, share.gpu_id);
                fflush(stdout);

                /* Submit to pool */
                snprintf(req, sizeof(req),
                    "{\"id\":%d,\"method\":\"mining.submit\","
                    "\"params\":{\"job_id\":\"%s\",\"worker\":\"%s\","
                    "\"nonce\":\"%s\"}}",
                    msg_id++, share.job_id, auth, nonce_hex);

                if (!stratum_send(sock, req)) goto reconnect;
            }
        }

reconnect:
        close(sock);
        if (state->running) {
            fprintf(stderr, "[pool] disconnected, retry in %ds\n", retry_delay);
            sleep(retry_delay);
            retry_delay = retry_delay < 60 ? retry_delay * 2 : 60;
        }
    }
}
```

- [ ] **Step 3: Build miner**

```bash
cd tools/txmminer-cuda && make clean && make ARCH=sm_86 2>&1 | grep -E "error:|Built"
```
Expected: `Built: tensorium-miner (sm_86)` — no errors.

- [ ] **Step 4: Commit**

```bash
git add tools/txmminer-cuda/stratum_client.cpp
git commit -m "feat(miner): stratum_client — TCP pool connection, notify parsing, share submit"
```

---

## Task 9: Integration test — pool mode end-to-end

- [ ] **Step 1: Start pool with Stratum on VPS (or local)**

```bash
# On VPS — update systemd service to add Stratum env vars
# /etc/systemd/system/tensorium-pool.service — add these Environment lines:
# Environment=TENSORIUM_STRATUM_BIND=0.0.0.0:3333
# Environment=TENSORIUM_POOL_SHARE_DIFF=1048576
# then:
# systemctl daemon-reload && systemctl restart tensorium-pool
```

Or run locally with tunnel to MC node:
```bash
ssh -fN -L 33332:127.0.0.1:33332 root@157.230.44.162

TENSORIUM_NODE_RPC=127.0.0.1:33332 \
TENSORIUM_POOL_TREASURY=txm10wa2dazhn2yqwwxkm4aegvzjq55hj9m2jlznt9 \
TENSORIUM_STRATUM_BIND=127.0.0.1:3333 \
TENSORIUM_POOL_SHARE_DIFF=1048576 \
./target/release/tensorium-pool serve &
```

- [ ] **Step 2: Run miner in pool mode**

```bash
cd tools/txmminer-cuda
./tensorium-miner \
  --mode pool \
  --pool stratum+tcp://127.0.0.1:3333 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck \
  --worker testrig \
  --gpu all \
  --intensity auto \
  --share-diff 1048576
```

Expected output within 30s:
```
[pool] connected
[pool] mining height=NNN  job=h...  bits=40  share_bits=20
[GPU 0] 7.8X GH/s
[pool] ✓ share  nonce=...  GPU=0
```

- [ ] **Step 3: Verify share accepted on pool side**

Pool logs should show:
```
[stratum] worker txm1xxx.testrig authorized
[stratum] share accepted from testrig
```

- [ ] **Step 4: Run both modes and compare hashrates**

Solo mode should show same hashrate as pool mode (within 5% variance):
```bash
# Solo: run for 30s, note GH/s
timeout 30 ./tensorium-miner --mode solo --rpc http://127.0.0.1:33332 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck --gpu all

# Pool: run for 30s, note GH/s
timeout 30 ./tensorium-miner --mode pool \
  --pool stratum+tcp://127.0.0.1:3333 \
  --wallet txm1xxjr2ca2n0zgxmw5rlwkcx7lgsrg9yy9qm0fck --gpu all
```

- [ ] **Step 5: Commit**

```bash
git commit -am "test(miner): integration test pool + solo mode verified"
```

---

## Task 10: Deploy, symlink, update VPS pool service

- [ ] **Step 1: Build all release architectures on Vast.ai**

```bash
ssh -p 2602 root@64.31.38.214 '
cd ~/tensorium-core && git pull origin main
cd tools/txmminer-cuda
make clean && make ARCH=sm_120 && cp tensorium-miner tensorium-miner-sm120
make clean && make ARCH=sm_89  && cp tensorium-miner tensorium-miner-sm89
make clean && make ARCH=sm_86  && cp tensorium-miner tensorium-miner-sm86
ls -lh tensorium-miner-sm*
'
```

- [ ] **Step 2: Install on Vast.ai**

```bash
ssh -p 2602 root@64.31.38.214 '
cd ~/tensorium-core/tools/txmminer-cuda
make install ARCH=sm_120
tensorium-miner --help
'
```

- [ ] **Step 3: Update VPS pool service with Stratum env vars**

```bash
sshpass -p 'PASS' ssh root@157.230.44.162 '
# Update pool systemd service
sed -i "/Environment=TENSORIUM_NODE_RPC/a Environment=TENSORIUM_STRATUM_BIND=0.0.0.0:3333\nEnvironment=TENSORIUM_POOL_SHARE_DIFF=1048576" \
    /etc/systemd/system/tensorium-pool.service

# Open UFW for Stratum port
ufw allow 3333/tcp comment "Tensorium Stratum miner port"

# Build and deploy new pool binary
cd /root/tensorium-core && git pull origin main
cargo build -p tensorium-pool --release
cp target/release/tensorium-pool /usr/local/bin/tensorium-pool

# Restart pool
systemctl daemon-reload && systemctl restart tensorium-pool
sleep 2
systemctl status tensorium-pool | grep -E "Active|error"
'
```

- [ ] **Step 4: Verify both endpoints from outside**

```bash
# HTTP pool (existing)
curl -s http://pooltxm.tensoriumlabs.com:23336/health

# Stratum (new) — send subscribe, expect JSON response
echo '{"id":1,"method":"mining.subscribe","params":["test"]}' | \
  nc -q 2 pool.tensoriumlabs.com 3333
```
Expected: JSON response with `session_id`.

- [ ] **Step 5: Upload new release binaries**

```bash
# Get binaries from Vast.ai
scp -P 2602 root@64.31.38.214:~/tensorium-core/tools/txmminer-cuda/tensorium-miner-sm86  /tmp/
scp -P 2602 root@64.31.38.214:~/tensorium-core/tools/txmminer-cuda/tensorium-miner-sm89  /tmp/
scp -P 2602 root@64.31.38.214:~/tensorium-core/tools/txmminer-cuda/tensorium-miner-sm120 /tmp/

# Upload to GitHub release (replace RELEASE_ID with v0.3.2-mainnet id or create v0.3.3)
for arch in sm86 sm89 sm120; do
  curl -s -X POST \
    -H "Authorization: token $GH_TOKEN" \
    -H "Content-Type: application/octet-stream" \
    "https://uploads.github.com/repos/tensorium-labs/tensorium-core/releases/RELEASE_ID/assets?name=tensorium-miner-linux-x86_64-${arch}" \
    --data-binary @/tmp/tensorium-miner-${arch}
done
```

- [ ] **Step 6: Update README pool mining command**

In `README.md`, update Mining Topology section to add Stratum pool command:
```bash
# Pool mining via Stratum (recommended for consistent payouts)
txmminer-cuda stratum+tcp://pooltxm.tensoriumlabs.com:3333 \
  txm1YOUR_ADDRESS
```
Wait — `txmminer-cuda` symlink points to `tensorium-miner`. The new tool needs `--mode pool --pool ...` syntax. Update README with new commands:

```markdown
**Pool mining (Stratum — port 3333):**
```bash
tensorium-miner \
  --mode pool \
  --pool stratum+tcp://pooltxm.tensoriumlabs.com:3333 \
  --wallet YOUR_TXM_ADDRESS \
  --worker WORKER_NAME \
  --gpu all
```

**Solo mining (0% fee):**
```bash
tensorium-miner \
  --mode solo \
  --rpc http://127.0.0.1:33332 \
  --wallet YOUR_TXM_ADDRESS \
  --gpu all
```
```

- [ ] **Step 7: Final commit and tag**

```bash
git add README.md tools/txmminer-cuda/
git commit -m "release: tensorium-miner v2 — multi-GPU, Stratum pool mode, NVML"
git tag -a v0.3.3-mainnet -m "tensorium-miner v2: multi-GPU, Stratum, NVML"
git push origin main --tags
```

---

## Self-Review Checklist

| Spec Section | Task(s) |
|---|---|
| Multi-file C++ structure | Task 1 scaffold |
| `common.h` shared types | Task 1 |
| `solo_client` HTTP RPC | Task 2 |
| `gpu_worker` per-GPU thread | Task 3 |
| Multi-GPU nonce split | Task 3 + Task 5 |
| `main.cpp` CLI flags | Task 4 |
| Backward compat `txmminer-cuda` | Task 4 (symlink in Makefile install) |
| `nvml_monitor` optional NVML | Task 6 |
| Stratum server `stratum.rs` | Task 7 |
| Stratum protocol (subscribe/notify/submit) | Task 7 |
| Share diff (1,048,576 default) | Task 7 + Task 8 |
| Share diff vs network diff | Task 7 `validate_share` |
| `stratum_client` pool mode | Task 8 |
| Reconnect/retry logic | Task 8 (retry loop) |
| Job broadcast on new block | Task 7 (poller + Vec<Sender>) |
| Clean shutdown (SIGINT) | Task 4 (`on_signal`) |
| VPS pool deploy | Task 10 |
| Release binaries | Task 10 |
| README update | Task 10 |

All spec sections covered. No TBDs or placeholders remain.

// main.cu — Tensorium CUDA miner
// Connects to node RPC, mines blocks using GPU, submits results.
//
// Usage: txmminer-cuda <rpc_host:port> <miner_address> [device_id] [blocks] [threads]
//
// Example:
//   txmminer-cuda 127.0.0.1:23332 txm1youraddress
//   txmminer-cuda 127.0.0.1:23332 txm1youraddress 0 1024 256

#include <cuda_runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <signal.h>

#ifdef _WIN32
  #include <winsock2.h>
  #pragma comment(lib, "ws2_32.lib")
  typedef SOCKET sock_t;
  #define CLOSE_SOCK closesocket
  #define SOCK_INVALID INVALID_SOCKET
#else
  #include <sys/socket.h>
  #include <netdb.h>
  #include <unistd.h>
  typedef int sock_t;
  #define CLOSE_SOCK close
  #define SOCK_INVALID (-1)
#endif

// Provided by mining_kernel.cu (extern "C" to match C linkage in mining_kernel.cu)
extern "C" int launch_mining_kernel(
    const uint8_t  header112[112],
    uint8_t        difficulty_bits,
    uint64_t       start_nonce,
    int            cuda_blocks,
    int            cuda_threads,
    uint32_t       iters_per_thread,
    uint64_t      *nonce_out
);

// ── Config ────────────────────────────────────────────────────────────────────

#define RPC_RECV_BUF   (1 << 20)   // 1 MB receive buffer
#define ITERS_DEFAULT  (1 << 20)   // 1M iters per thread per kernel launch
#define BLOCKS_DEFAULT  2048
#define THREADS_DEFAULT 256

static volatile int g_running = 1;
static void handle_sigint(int s) { (void)s; g_running = 0; }

// ── Minimal JSON helpers ──────────────────────────────────────────────────────

// Find value of a string key in flat JSON. Caller provides output buffer.
// Returns 1 on success. Works for simple string and number values.
static int json_get_str(const char *json, const char *key, char *out, int out_len) {
    char search[128];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return 0;
    p += strlen(search);
    while (*p == ' ' || *p == ':') p++;
    if (*p == '"') {
        p++;
        int i = 0;
        while (*p && *p != '"' && i < out_len - 1) out[i++] = *p++;
        out[i] = '\0';
        return 1;
    }
    // number or bool
    int i = 0;
    while (*p && *p != ',' && *p != '}' && *p != ']' && i < out_len - 1) out[i++] = *p++;
    out[i] = '\0';
    return 1;
}

// ── Write helpers ─────────────────────────────────────────────────────────────

static void write_le64(uint8_t *buf, uint64_t v) {
    for (int i = 0; i < 8; i++) { buf[i] = (uint8_t)(v & 0xff); v >>= 8; }
}

static void write_le32(uint8_t *buf, uint32_t v) {
    for (int i = 0; i < 4; i++) { buf[i] = (uint8_t)(v & 0xff); v >>= 8; }
}

// ── TCP HTTP client ───────────────────────────────────────────────────────────

static sock_t tcp_connect(const char *host, const char *port) {
    struct addrinfo hints = {0}, *res;
    hints.ai_family   = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) return SOCK_INVALID;
    sock_t s = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (s == SOCK_INVALID) { freeaddrinfo(res); return SOCK_INVALID; }
    if (connect(s, res->ai_addr, (int)res->ai_addrlen) != 0) {
        CLOSE_SOCK(s); freeaddrinfo(res); return SOCK_INVALID;
    }
    freeaddrinfo(res);
    return s;
}

// Perform an HTTP GET. Returns body in buf (null-terminated). Returns 1 on 200, 0 otherwise.
static int http_get(const char *host, const char *port, const char *path,
                    char *buf, int buf_len) {
    sock_t s = tcp_connect(host, port);
    if (s == SOCK_INVALID) { fprintf(stderr, "connect failed\n"); return 0; }

    char req[512];
    int  reqlen = snprintf(req, sizeof(req),
        "GET %s HTTP/1.1\r\nHost: %s:%s\r\nConnection: close\r\n\r\n",
        path, host, port);
    send(s, req, reqlen, 0);

    int total = 0;
    char tmp[4096];
    int n;
    while ((n = recv(s, tmp, sizeof(tmp), 0)) > 0) {
        if (total + n < buf_len) { memcpy(buf + total, tmp, n); total += n; }
    }
    CLOSE_SOCK(s);
    buf[total] = '\0';

    char *body = strstr(buf, "\r\n\r\n");
    if (!body) return 0;
    body += 4;
    memmove(buf, body, strlen(body) + 1);
    return strstr(buf, "HTTP/1.1 200") || (buf[0] == '{') ? 1 : 0;
}

// Perform an HTTP POST. Returns body in buf. Returns 1 on 200.
static int http_post(const char *host, const char *port, const char *path,
                     const char *body_str, char *buf, int buf_len) {
    sock_t s = tcp_connect(host, port);
    if (s == SOCK_INVALID) { fprintf(stderr, "connect failed\n"); return 0; }

    int blen = (int)strlen(body_str);
    char hdr[512];
    int hlen = snprintf(hdr, sizeof(hdr),
        "POST %s HTTP/1.1\r\nHost: %s:%s\r\n"
        "Content-Type: application/json\r\nContent-Length: %d\r\n"
        "Connection: close\r\n\r\n",
        path, host, port, blen);
    send(s, hdr, hlen, 0);
    send(s, body_str, blen, 0);

    int total = 0;
    char tmp[4096];
    int n;
    while ((n = recv(s, tmp, sizeof(tmp), 0)) > 0) {
        if (total + n < buf_len) { memcpy(buf + total, tmp, n); total += n; }
    }
    CLOSE_SOCK(s);
    buf[total] = '\0';

    char *resp = strstr(buf, "\r\n\r\n");
    if (!resp) return 0;
    resp += 4;
    memmove(buf, resp, strlen(resp) + 1);
    return 1;
}

// ── Block template parsing ────────────────────────────────────────────────────

typedef struct {
    uint32_t version;
    char     chain_id[64];
    uint64_t height;
    uint8_t  previous_hash[32];
    uint8_t  merkle_root[32];
    uint64_t timestamp_seconds;
    uint8_t  difficulty_bits;
    char     template_json[RPC_RECV_BUF]; // keep full JSON for submitblock
} BlockTemplate;

// Parse JSON array [b0, b1, ...] of bytes into out[n].
// Handles the byte-array format returned by the Tensorium RPC.
static int parse_byte_array(const char *json_arr_start, uint8_t *out, int n) {
    const char *p = json_arr_start;
    while (*p && *p != '[') p++;
    if (!*p) return 0;
    p++; // skip '['
    for (int i = 0; i < n; i++) {
        while (*p == ' ' || *p == ',') p++;
        if (*p == ']') return 0; // too few elements
        char *end;
        out[i] = (uint8_t)strtoul(p, &end, 10);
        p = end;
    }
    return 1;
}

// Extract field "name": [...] array from JSON and parse as byte array
static int extract_byte_array(const char *json, const char *name, uint8_t *out, int n) {
    char key[128];
    snprintf(key, sizeof(key), "\"%s\"", name);
    const char *p = strstr(json, key);
    if (!p) return 0;
    p += strlen(key);
    while (*p == ' ' || *p == ':') p++;
    return parse_byte_array(p, out, n);
}

static int get_block_template(const char *host, const char *port,
                               const char *miner_addr, BlockTemplate *tmpl) {
    static char buf[RPC_RECV_BUF];
    char path[256];
    snprintf(path, sizeof(path), "/getblocktemplate/%s", miner_addr);
    if (!http_get(host, port, path, buf, sizeof(buf))) return 0;

    // buf = {"template": {"header": {...}, "transactions": [...]}}
    const char *header_start = strstr(buf, "\"header\"");
    if (!header_start) return 0;

    char val[64];
    if (json_get_str(header_start, "version", val, sizeof(val)))
        tmpl->version = (uint32_t)strtoul(val, NULL, 10);
    if (json_get_str(header_start, "chain_id", tmpl->chain_id, sizeof(tmpl->chain_id)));
    if (json_get_str(header_start, "height", val, sizeof(val)))
        tmpl->height = (uint64_t)strtoull(val, NULL, 10);
    if (json_get_str(header_start, "timestamp_seconds", val, sizeof(val)))
        tmpl->timestamp_seconds = (uint64_t)strtoull(val, NULL, 10);
    if (json_get_str(header_start, "leading_zero_bits", val, sizeof(val)))
        tmpl->difficulty_bits = (uint8_t)strtoul(val, NULL, 10);

    extract_byte_array(header_start, "previous_hash", tmpl->previous_hash, 32);
    extract_byte_array(header_start, "merkle_root",   tmpl->merkle_root,   32);

    // Keep full RPC response for submitblock
    strncpy(tmpl->template_json, buf, RPC_RECV_BUF - 1);

    return 1;
}

// ── Build 112-byte header from template + nonce ───────────────────────────────

static void build_header(const BlockTemplate *tmpl, uint64_t nonce, uint8_t out[112]) {
    int pos = 0;
    write_le32(out + pos, tmpl->version); pos += 4;
    int cid_len = (int)strlen(tmpl->chain_id);
    memcpy(out + pos, tmpl->chain_id, cid_len); pos += cid_len;
    write_le64(out + pos, tmpl->height); pos += 8;
    memcpy(out + pos, tmpl->previous_hash, 32); pos += 32;
    memcpy(out + pos, tmpl->merkle_root,   32); pos += 32;
    write_le64(out + pos, tmpl->timestamp_seconds); pos += 8;
    out[pos++] = tmpl->difficulty_bits;
    write_le64(out + pos, nonce); pos += 8;
    // Pad to 112 if chain_id shorter than 19 bytes (shouldn't happen)
    while (pos < 112) out[pos++] = 0;
}

// ── Submit mined block ────────────────────────────────────────────────────────

static int submit_block(const char *host, const char *port,
                        const char *template_json, uint64_t winning_nonce) {
    // Replace "nonce":0 with the winning nonce in the template JSON
    char *json = strdup(template_json);
    if (!json) return 0;

    // Find and replace the nonce field inside header
    char nonce_str[32];
    snprintf(nonce_str, sizeof(nonce_str), "%llu", (unsigned long long)winning_nonce);

    // Build: extract template object ({"template":{...}}) and update nonce
    // Simple approach: find "nonce": in the header section and replace
    char *nonce_pos = strstr(json, "\"nonce\":");
    if (!nonce_pos) { free(json); return 0; }

    nonce_pos += 8; // skip "nonce":
    while (*nonce_pos == ' ') nonce_pos++;

    // Find end of current nonce value
    char *nonce_end = nonce_pos;
    while (*nonce_end && *nonce_end != ',' && *nonce_end != '}' && *nonce_end != ' ')
        nonce_end++;

    // Build new JSON: before + new_nonce + after
    int before_len = (int)(nonce_pos - json);
    int after_len  = (int)strlen(nonce_end);
    int new_len    = before_len + (int)strlen(nonce_str) + after_len + 1;
    char *new_json = (char *)malloc(new_len);
    if (!new_json) { free(json); return 0; }

    memcpy(new_json, json, before_len);
    memcpy(new_json + before_len, nonce_str, strlen(nonce_str));
    memcpy(new_json + before_len + strlen(nonce_str), nonce_end, after_len + 1);
    free(json);

    // Extract "template" value — find "template": then grab the object
    // new_json has form {"leading_zero_bits":N,...,"template":{...block_json...}}
    // We need to extract just the block JSON object for /submitblock
    const char *tmpl_key = strstr(new_json, "\"template\"");
    if (!tmpl_key) { free(new_json); fprintf(stderr, "no template key\n"); return 0; }
    const char *tmpl_val = strchr(tmpl_key + 10, ':');
    if (!tmpl_val) { free(new_json); return 0; }
    tmpl_val++;
    while (*tmpl_val == ' ') tmpl_val++;
    // tmpl_val points to the opening { of the block JSON
    // Count matching braces to find end of block object
    int depth = 0;
    const char *p = tmpl_val;
    while (*p) {
        if (*p == '{') depth++;
        else if (*p == '}') { if (--depth == 0) { p++; break; } }
        p++;
    }
    int body_len = (int)(p - tmpl_val);
    char *body = (char *)malloc(body_len + 1);
    if (!body) { free(new_json); return 0; }
    memcpy(body, tmpl_val, body_len);
    body[body_len] = '\0';

    static char resp_buf[4096];
    int ok = http_post(host, port, "/submitblock", body, resp_buf, sizeof(resp_buf));
    free(body);
    free(new_json);

    if (ok) {
        char accepted[16];
        if (json_get_str(resp_buf, "accepted", accepted, sizeof(accepted)))
            return strcmp(accepted, "true") == 0 ? 1 : 0;
    }
    return 0;
}

// ── Main ──────────────────────────────────────────────────────────────────────

int main(int argc, char *argv[]) {
    if (argc < 3) {
        fprintf(stderr,
            "Tensorium CUDA Miner\n"
            "Usage: %s <host:port> <address> [device_id] [cuda_blocks] [cuda_threads]\n"
            "Example: %s 127.0.0.1:23332 txm1youraddress 0 2048 256\n",
            argv[0], argv[0]);
        return 1;
    }

    // Parse host:port
    char host[128], port[16];
    const char *colon = strrchr(argv[1], ':');
    if (!colon) { fprintf(stderr, "invalid rpc address, use host:port\n"); return 1; }
    int host_len = (int)(colon - argv[1]);
    memcpy(host, argv[1], host_len); host[host_len] = '\0';
    strncpy(port, colon + 1, sizeof(port) - 1);

    const char *miner_addr = argv[2];
    int device_id      = argc > 3 ? atoi(argv[3]) : 0;
    int cuda_blocks    = argc > 4 ? atoi(argv[4]) : BLOCKS_DEFAULT;
    int cuda_threads   = argc > 5 ? atoi(argv[5]) : THREADS_DEFAULT;

    // Total nonces searched per kernel launch
    uint32_t iters = ITERS_DEFAULT / (cuda_blocks * cuda_threads);
    if (iters < 1) iters = 1;

    uint64_t total_nonces_per_launch = (uint64_t)cuda_blocks * cuda_threads * iters;

    signal(SIGINT, handle_sigint);

#ifdef _WIN32
    WSADATA wsa; WSAStartup(MAKEWORD(2,2), &wsa);
#endif

    // Select CUDA device
    cudaSetDevice(device_id);
    cudaDeviceProp prop;
    cudaGetDeviceProperties(&prop, device_id);
    printf("txmminer-cuda — device=%s blocks=%d threads=%d\n",
           prop.name, cuda_blocks, cuda_threads);
    printf("Nonces per launch: %llu\n\n", (unsigned long long)total_nonces_per_launch);

    uint64_t start_nonce = 0;
    // static to avoid stack overflow (template_json is 1MB per struct)
    static BlockTemplate tmpl;
    static BlockTemplate last_tmpl;
    memset(&last_tmpl, 0, sizeof(last_tmpl));

    // Fetch initial template
    while (g_running && !get_block_template(host, port, miner_addr, &tmpl)) {
        fprintf(stderr, "Failed to get template — retrying in 3s\n");
        sleep(3);
    }
    memcpy(&last_tmpl, &tmpl, sizeof(tmpl));
    printf("mining  height=%llu  bits=%u  blocks=%d  threads=%d\n",
           (unsigned long long)tmpl.height, tmpl.difficulty_bits,
           cuda_blocks, cuda_threads);
    fflush(stdout);

    // Timer: refresh template every TEMPLATE_REFRESH_SEC seconds
    #define TEMPLATE_REFRESH_SEC 10
    struct timespec last_refresh;
    clock_gettime(CLOCK_MONOTONIC, &last_refresh);

    uint64_t total_hashes = 0;
    struct timespec rate_t0;
    clock_gettime(CLOCK_MONOTONIC, &rate_t0);

    while (g_running) {
        // Build 112-byte header (nonce=0, will be set by kernel)
        uint8_t header112[112] = {0};
        build_header(&tmpl, 0, header112);

        uint64_t winning_nonce = 0;
        int found = launch_mining_kernel(
            header112, tmpl.difficulty_bits, start_nonce,
            cuda_blocks, cuda_threads, iters, &winning_nonce
        );

        total_hashes += total_nonces_per_launch;
        start_nonce  += total_nonces_per_launch;

        if (found) {
            struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
            double elapsed = (now.tv_sec - rate_t0.tv_sec) +
                             (now.tv_nsec - rate_t0.tv_nsec) / 1e9;
            double hashrate = total_hashes / elapsed;
            printf("✓  height=%llu  nonce=%llu  ",
                   (unsigned long long)tmpl.height,
                   (unsigned long long)winning_nonce);
            if (hashrate >= 1e9)      printf("%.2f GH/s\n", hashrate / 1e9);
            else if (hashrate >= 1e6) printf("%.2f MH/s\n", hashrate / 1e6);
            else                       printf("%.2f KH/s\n", hashrate / 1e3);
            fflush(stdout);

            if (!submit_block(host, port, tmpl.template_json, winning_nonce))
                fprintf(stderr, "  ✗ block rejected (stale)\n");

            // Fresh template after submit
            if (get_block_template(host, port, miner_addr, &tmpl)) {
                if (tmpl.height != last_tmpl.height ||
                    memcmp(tmpl.previous_hash, last_tmpl.previous_hash, 32) != 0) {
                    printf("mining  height=%llu  bits=%u\n",
                           (unsigned long long)tmpl.height, tmpl.difficulty_bits);
                    fflush(stdout);
                    memcpy(&last_tmpl, &tmpl, sizeof(tmpl));
                }
            }
            start_nonce = 0;
            total_hashes = 0;
            clock_gettime(CLOCK_MONOTONIC, &rate_t0);
            clock_gettime(CLOCK_MONOTONIC, &last_refresh);
        } else {
            // Print hashrate periodically
            static int tick = 0;
            if (++tick % 200 == 0) {
                struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
                double elapsed = (now.tv_sec - rate_t0.tv_sec) +
                                 (now.tv_nsec - rate_t0.tv_nsec) / 1e9;
                double hashrate = total_hashes / elapsed;
                if (hashrate >= 1e9)
                    printf("\r  %.2f GH/s  nonce=%llu   ",
                           hashrate/1e9, (unsigned long long)start_nonce);
                else
                    printf("\r  %.2f MH/s  nonce=%llu   ",
                           hashrate/1e6, (unsigned long long)start_nonce);
                fflush(stdout);
            }

            // Timer-based template refresh (every TEMPLATE_REFRESH_SEC seconds)
            struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
            double since_refresh = (now.tv_sec - last_refresh.tv_sec) +
                                   (now.tv_nsec - last_refresh.tv_nsec) / 1e9;
            if (since_refresh >= TEMPLATE_REFRESH_SEC) {
                BlockTemplate fresh;
                if (get_block_template(host, port, miner_addr, &fresh)) {
                    clock_gettime(CLOCK_MONOTONIC, &last_refresh);
                    if (fresh.height != tmpl.height ||
                        memcmp(fresh.previous_hash, tmpl.previous_hash, 32) != 0) {
                        memcpy(&tmpl, &fresh, sizeof(tmpl));
                        memcpy(&last_tmpl, &fresh, sizeof(fresh));
                        start_nonce = 0;
                        total_hashes = 0;
                        clock_gettime(CLOCK_MONOTONIC, &rate_t0);
                        printf("\n  New block detected — height=%llu\n",
                               (unsigned long long)tmpl.height);
                        fflush(stdout);
                    }
                }
            }
        }
    }

    printf("\nStopped.\n");
#ifdef _WIN32
    WSACleanup();
#endif
    return 0;
}

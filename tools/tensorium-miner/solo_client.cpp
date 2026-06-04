// tools/tensorium-miner/solo_client.cpp
#include "solo_client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netdb.h>
#include <time.h>

#define RPC_BUF (1 << 20)   /* 1 MB */

// ── TCP helpers ──────────────────────────────────────────────────────────────

static int tcp_connect(const char *host, const char *port) {
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
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

// ── JSON helpers ──────────────────────────────────────────────────────────────

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

// ── Template fetch ─────────────────────────────────────────────────────────────

/* Global buf so stack doesn't overflow (template_json is 1 MB) */
static char s_rpc_buf[RPC_BUF];
/* Stores raw template JSON: {"header":{...},"transactions":[...]} for submitblock */
static char s_template_json[RPC_BUF];

static int fetch_template(const char *host, const char *port,
                           const char *wallet, JobDesc *job) {
    char path[256];
    snprintf(path, sizeof(path), "/getblocktemplate/%s", wallet);
    if (!http_get(host, port, path, s_rpc_buf, sizeof(s_rpc_buf))) return 0;

    /* Navigate into template.header */
    const char *hdr = strstr(s_rpc_buf, "\"header\"");
    if (!hdr) return 0;

    char val[64];
    if (!json_str(hdr, "chain_id", job->chain_id, CHAIN_ID_LEN)) return 0;

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

    snprintf(job->job_id, JOB_ID_LEN, "solo-%llu",
             (unsigned long long)job->height);
    job->valid = 1;

    /* Extract and cache raw template JSON for submitblock.
       The response has: {...,"template":{...},...}
       We need the value of "template" — the full Block JSON. */
    const char *tmpl_key = strstr(s_rpc_buf, "\"template\"");
    s_template_json[0] = '\0';
    if (tmpl_key) {
        const char *p = tmpl_key + strlen("\"template\"");
        while (*p == ' ' || *p == ':') p++;
        if (*p == '{') {
            /* Copy the full object by counting braces */
            int depth = 0;
            const char *start = p;
            const char *q = p;
            int in_str = 0;
            while (*q) {
                if (!in_str) {
                    if (*q == '{') depth++;
                    else if (*q == '}') { depth--; if (depth == 0) { q++; break; } }
                    else if (*q == '"') in_str = 1;
                } else {
                    if (*q == '"' && *(q-1) != '\\') in_str = 0;
                }
                q++;
            }
            int len = (int)(q - start);
            if (len > 0 && len < RPC_BUF - 1) {
                memcpy(s_template_json, start, len);
                s_template_json[len] = '\0';
            }
        }
    }

    return 1;
}

// ── Block submit ───────────────────────────────────────────────────────────────

static int submit_block(const char *host, const char *port,
                        const JobDesc *job, uint64_t nonce) {
    /* Submit the cached template JSON with nonce replaced.
       s_template_json = {"header":{...,"nonce":0,...},"transactions":[...]}
       /submitblock expects this exact Block format. */
    if (s_template_json[0] == '\0') {
        fprintf(stderr, "[solo] no cached template for submitblock\n");
        return 0;
    }

    char *nonce_pos = strstr(s_template_json, "\"nonce\"");
    if (!nonce_pos) {
        fprintf(stderr, "[solo] nonce field not found in template\n");
        return 0;
    }

    FILE *f = fopen("/tmp/txm_submit.json", "w");
    if (!f) return 0;

    /* Write everything up to "nonce" */
    int prefix = (int)(nonce_pos - s_template_json);
    fwrite(s_template_json, 1, prefix, f);

    /* Write new nonce */
    fprintf(f, "\"nonce\": %llu", (unsigned long long)nonce);

    /* Skip the old nonce value and write the rest */
    const char *after = nonce_pos + strlen("\"nonce\"");
    while (*after == ' ' || *after == ':') after++;
    while (*after && (*after == '-' || (*after >= '0' && *after <= '9'))) after++;
    fputs(after, f);
    fclose(f);

    char cmd[512];
    snprintf(cmd, sizeof(cmd),
        "curl -s -X POST http://%s:%s/submitblock "
        "-H \"Content-Type: application/json\" -d @/tmp/txm_submit.json",
        host, port);
    FILE *p = popen(cmd, "r");
    if (!p) return 0;
    char resp[512] = {0};
    fread(resp, 1, sizeof(resp) - 1, p);
    pclose(p);
    printf("[solo] submitblock response: %.80s\n", resp);
    fflush(stdout);
    return strstr(resp, "\"accepted\"") ? 1 : 0;
}

// ── build_header ───────────────────────────────────────────────────────────────

static void write_le64(uint8_t *b, uint64_t v) {
    for (int i = 0; i < 8; i++) { b[i] = v & 0xff; v >>= 8; }
}
static void write_le32(uint8_t *b, uint32_t v) {
    for (int i = 0; i < 4; i++) { b[i] = v & 0xff; v >>= 8; }
}

int build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]) {
    int cid_len = (int)strlen(job->chain_id);
    if (cid_len <= 0) return 0;
    int total = 4 + cid_len + 8 + 32 + 32 + 8 + 1 + 8;
    if (total > HEADER_MAX) return 0;

    int pos = 0;
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

// ── solo_client_run ────────────────────────────────────────────────────────────

void solo_client_run(const MinerConfig *cfg, SharedState *state) {
    const char *host = cfg->rpc_host;
    const char *port = cfg->rpc_port;

    JobDesc job;
    memset(&job, 0, sizeof(job));
    /* Solo mode: mine at full network difficulty — no pool, so share_bits = difficulty_bits */
    job.share_bits = 0; /* will be set from difficulty_bits after fetch */

    /* Fetch first template with retries */
    while (state->running && !fetch_template(host, port, cfg->wallet, &job)) {
        fprintf(stderr, "[solo] failed to get template — retrying in 3s\n");
        fflush(stderr);
        sleep(3);
    }
    if (!state->running) return;

    /* In solo mode the kernel must mine at full 40-bit network difficulty.
       Setting share_bits = difficulty_bits means every found nonce IS a real block. */
    job.share_bits = job.difficulty_bits;
    job_publish(state, &job);
    printf("[solo] height=%llu  bits=%u  (solo: kernel mines at full difficulty)\n",
           (unsigned long long)job.height,
           job.difficulty_bits);
    fflush(stdout);

    time_t last_refresh = time(NULL);
    uint64_t last_height = job.height;

    while (state->running) {
        /* Try to pop shares from GPU workers and handle them */
        ShareResult share;
        if (share_pop(state, &share)) {
            if (share.is_block) {
                printf("[solo] ⛏ BLOCK FOUND! height=%llu  nonce=%llu  GPU=%d\n",
                       (unsigned long long)job.height,
                       (unsigned long long)share.nonce, share.gpu_id);
                fflush(stdout);
                if (submit_block(host, port, &job, share.nonce))
                    printf("[solo] block submitted OK — fetching new template\n");
                else
                    fprintf(stderr, "[solo] block submission failed\n");
                fflush(stdout);

                /* Force immediate template refresh so GPU stops re-mining same nonce */
                JobDesc fresh;
                memset(&fresh, 0, sizeof(fresh));
                for (int r = 0; r < 5 && state->running; r++) {
                    if (fetch_template(host, port, cfg->wallet, &fresh)) {
                        fresh.share_bits = fresh.difficulty_bits;
                        job = fresh;
                        last_height = job.height;
                        last_refresh = time(NULL);
                        job_publish(state, &job);
                        printf("[solo] mining height=%llu  bits=%u\n",
                               (unsigned long long)job.height, job.difficulty_bits);
                        fflush(stdout);
                        break;
                    }
                    sleep(1);
                }
            }
            /* Solo mode: non-block shares are discarded */
        }

        /* Refresh template every 10s or if chain advanced */
        if (time(NULL) - last_refresh >= 10) {
            JobDesc fresh;
            memset(&fresh, 0, sizeof(fresh));
            if (fetch_template(host, port, cfg->wallet, &fresh)) {
                if (fresh.height != last_height ||
                    memcmp(fresh.previous_hash, job.previous_hash, 32) != 0) {
                    fresh.share_bits = fresh.difficulty_bits;
                    job = fresh;
                    last_height = job.height;
                    job_publish(state, &job);
                    printf("[solo] new block  height=%llu\n",
                           (unsigned long long)job.height);
                    fflush(stdout);
                }
            }
            last_refresh = time(NULL);
        }
    }
}

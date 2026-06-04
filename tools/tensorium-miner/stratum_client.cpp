// tools/tensorium-miner/stratum_client.cpp
#include "stratum_client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netdb.h>
#include <time.h>

#define STRATUM_LINEBUF 65536

// ── TCP ───────────────────────────────────────────────────────────────────────

static int stratum_connect(const char *host, const char *port) {
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
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

static int stratum_sendline(int sock, const char *json) {
    char line[STRATUM_LINEBUF];
    int n = snprintf(line, sizeof(line) - 1, "%s\n", json);
    return (int)send(sock, line, n, 0) == n ? 1 : 0;
}

/* Read one '\n'-terminated line. Returns 1 on success, 0 on disconnect/error. */
static int stratum_readline(int sock, char *buf, int buf_len) {
    int pos = 0;
    char c;
    while (pos < buf_len - 1) {
        int n = (int)recv(sock, &c, 1, 0);
        if (n <= 0) return 0;
        if (c == '\n') break;
        buf[pos++] = c;
    }
    buf[pos] = '\0';
    return pos > 0 ? 1 : 0;
}

// ── Minimal JSON field extractors ──────────────────────────────────────────────

static int jstr(const char *json, const char *key, char *out, int len) {
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

static uint64_t jnum(const char *json, const char *key) {
    char val[32] = {0};
    jstr(json, key, val, sizeof(val));
    return val[0] ? (uint64_t)strtoull(val, NULL, 10) : 0;
}

/* Parse 64-char hex string into 32-byte array */
static void hex64_to_bytes(const char *hex, uint8_t out[32]) {
    for (int i = 0; i < 32; i++) {
        int hi = hex[i*2];
        int lo = hex[i*2+1];
        hi = (hi >= 'a') ? hi-'a'+10 : (hi >= 'A') ? hi-'A'+10 : hi-'0';
        lo = (lo >= 'a') ? lo-'a'+10 : (lo >= 'A') ? lo-'A'+10 : lo-'0';
        out[i] = (uint8_t)((hi << 4) | lo);
    }
}

/* Nonce → 16-char lowercase hex string (little-endian byte order) */
static void nonce_to_hex(uint64_t nonce, char out[17]) {
    /* Store as 8 LE bytes then hex-encode */
    uint8_t b[8];
    for (int i = 0; i < 8; i++) { b[i] = nonce & 0xff; nonce >>= 8; }
    snprintf(out, 17, "%02x%02x%02x%02x%02x%02x%02x%02x",
             b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7]);
}

// ── Parse mining.notify → JobDesc ─────────────────────────────────────────────

/* Navigate into params object inside the line */
static const char *find_params(const char *line) {
    const char *p = strstr(line, "\"params\"");
    if (!p) return NULL;
    p += strlen("\"params\"");
    while (*p == ' ' || *p == ':') p++;
    return p;
}

static int parse_notify(const char *line, JobDesc *job, uint64_t share_diff) {
    const char *params = find_params(line);
    if (!params) return 0;

    if (!jstr(params, "job_id",   job->job_id,   JOB_ID_LEN))    return 0;
    if (!jstr(params, "chain_id", job->chain_id, CHAIN_ID_LEN)) {
        strncpy(job->chain_id, "tensorium-mainnet-candidate-0", CHAIN_ID_LEN - 1);
    }
    job->height          = jnum(params, "height");
    job->timestamp       = jnum(params, "timestamp");
    job->difficulty_bits = (uint8_t)jnum(params, "difficulty_bits");
    job->version         = 1;

    uint64_t pool_diff = jnum(params, "share_difficulty");
    if (pool_diff > 0) share_diff = pool_diff;
    job->share_bits = share_bits_from_diff(share_diff);

    char hex[128] = {0};
    if (jstr(params, "previous_hash", hex, sizeof(hex)) && strlen(hex) == 64)
        hex64_to_bytes(hex, job->previous_hash);
    memset(hex, 0, sizeof(hex));
    if (jstr(params, "merkle_root", hex, sizeof(hex)) && strlen(hex) == 64)
        hex64_to_bytes(hex, job->merkle_root);

    job->valid = 1;
    return 1;
}

// ── stratum_client_run ─────────────────────────────────────────────────────────

void stratum_client_run(const MinerConfig *cfg, SharedState *state) {
    /* Build auth string: wallet.worker */
    char auth[ADDR_LEN + WORKER_LEN + 2];
    snprintf(auth, sizeof(auth), "%s.%s", cfg->wallet, cfg->worker);

    int retry_delay = 5;
    int msg_id      = 1;
    char req[512];

    while (state->running) {
        printf("[pool] connecting to %s:%s...\n",
               cfg->pool_host, cfg->pool_port);
        fflush(stdout);

        int sock = stratum_connect(cfg->pool_host, cfg->pool_port);
        if (sock < 0) {
            fprintf(stderr, "[pool] connect failed — retry in %ds\n", retry_delay);
            sleep(retry_delay);
            retry_delay = (retry_delay < 60) ? retry_delay * 2 : 60;
            continue;
        }
        retry_delay = 5; /* reset backoff on successful connect */
        msg_id = 1;

        /* Subscribe */
        snprintf(req, sizeof(req),
            "{\"id\":%d,\"method\":\"mining.subscribe\","
            "\"params\":[\"tensorium-miner/" TENSORIUM_MINER_VERSION "\"]}", msg_id++);
        if (!stratum_sendline(sock, req)) { close(sock); continue; }

        /* Authorize */
        snprintf(req, sizeof(req),
            "{\"id\":%d,\"method\":\"mining.authorize\","
            "\"params\":[\"%s\",\"x\"]}", msg_id++, auth);
        if (!stratum_sendline(sock, req)) { close(sock); continue; }

        uint64_t effective_diff = cfg->share_diff;
        char buf[STRATUM_LINEBUF];

        while (state->running) {
            /* Submit any pending shares first (non-blocking check) */
            ShareResult share;
            while (state->share_count > 0 && share_pop(state, &share)) {
                char nonce_hex[17];
                nonce_to_hex(share.nonce, nonce_hex);

                snprintf(req, sizeof(req),
                    "{\"id\":%d,\"method\":\"mining.submit\","
                    "\"params\":{\"job_id\":\"%s\",\"worker\":\"%s\","
                    "\"nonce\":\"%s\"}}",
                    msg_id++, share.job_id, auth, nonce_hex);

                printf("[pool] %s share  nonce=%s  GPU=%d\n",
                       share.is_block ? "⛏ BLOCK" : "✓",
                       nonce_hex, share.gpu_id);
                fflush(stdout);

                if (!stratum_sendline(sock, req)) goto reconnect;
            }

            /* Read one line from pool (blocking, up to SO_RCVTIMEO = 15s) */
            if (!stratum_readline(sock, buf, sizeof(buf))) goto reconnect;

            char method[64] = {0};
            jstr(buf, "method", method, sizeof(method));

            if (strcmp(method, "mining.notify") == 0) {
                JobDesc job;
                memset(&job, 0, sizeof(job));
                if (parse_notify(buf, &job, effective_diff)) {
                    job_publish(state, &job);
                    printf("[pool] mining height=%llu  job=%s  bits=%u  share_bits=%u\n",
                           (unsigned long long)job.height, job.job_id,
                           job.difficulty_bits, job.share_bits);
                    fflush(stdout);
                }

            } else if (strcmp(method, "mining.set_difficulty") == 0) {
                /* params is an array: [difficulty_number] */
                const char *p = strstr(buf, "\"params\"");
                if (p) {
                    p += strlen("\"params\"");
                    while (*p == ' ' || *p == ':') p++;
                    if (*p == '[') { p++; while (*p == ' ') p++; }
                    uint64_t d = (uint64_t)strtoull(p, NULL, 10);
                    if (d > 0) {
                        effective_diff = d;
                        printf("[pool] share_diff=%llu (~%u bits)\n",
                               (unsigned long long)effective_diff,
                               share_bits_from_diff(effective_diff));
                        fflush(stdout);
                    }
                }

            } else if (strcmp(method, "mining.ping") == 0) {
                stratum_sendline(sock, "{\"id\":null,\"method\":\"mining.pong\",\"params\":[]}");

            } else {
                /* Ignore other messages (subscribe/authorize responses, accepted/rejected) */
                char result[32] = {0};
                jstr(buf, "result", result, sizeof(result));
                if (strcmp(result, "rejected") == 0) {
                    char err[64] = {0};
                    jstr(buf, "error", err, sizeof(err));
                    fprintf(stderr, "[pool] share rejected: %s\n", err);
                    fflush(stderr);
                }
            }
        }
        goto done;

reconnect:
        close(sock);
        if (state->running) {
            fprintf(stderr, "[pool] disconnected — retry in %ds\n", retry_delay);
            sleep(retry_delay);
            retry_delay = (retry_delay < 60) ? retry_delay * 2 : 60;
        }
        continue;
    }
done:
    return;
}

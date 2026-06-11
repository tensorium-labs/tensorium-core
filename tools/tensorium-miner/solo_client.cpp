// tools/tensorium-miner/solo_client.cpp
#include "solo_client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netdb.h>
#include <time.h>
#include <openssl/ssl.h>
#include <openssl/err.h>

#define RPC_BUF (1 << 20)   /* 1 MB */

// ── TCP / TLS helpers ───────────────────────────────────────────────────────

/* Either a plain TCP socket (ssl == NULL) or a TLS connection (ssl != NULL,
   sock still owns the underlying fd for cleanup). */
typedef struct {
    int  sock;
    SSL *ssl;
    SSL_CTX *ctx;
} Conn;

typedef struct {
    int  status_code;
    int  header_len;
    char location[512];
} HttpMeta;

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

/* Opens a connection, performing a TLS handshake (with SNI) when use_tls. */
static int conn_open(const char *host, const char *port, int use_tls, Conn *c) {
    memset(c, 0, sizeof(*c));
    c->sock = tcp_connect(host, port);
    if (c->sock < 0) return 0;
    if (!use_tls) return 1;

    c->ctx = SSL_CTX_new(TLS_client_method());
    if (!c->ctx) { close(c->sock); return 0; }
    SSL_CTX_set_verify(c->ctx, SSL_VERIFY_NONE, NULL);

    c->ssl = SSL_new(c->ctx);
    if (!c->ssl) { SSL_CTX_free(c->ctx); close(c->sock); return 0; }
    SSL_set_tlsext_host_name(c->ssl, host); /* SNI */
    SSL_set_fd(c->ssl, c->sock);
    if (SSL_connect(c->ssl) != 1) {
        SSL_free(c->ssl); SSL_CTX_free(c->ctx); close(c->sock);
        return 0;
    }
    return 1;
}

static int conn_send(Conn *c, const char *buf, int len) {
    if (c->ssl) return SSL_write(c->ssl, buf, len);
    return (int)send(c->sock, buf, len, 0);
}

static int conn_recv(Conn *c, char *buf, int len) {
    if (c->ssl) return SSL_read(c->ssl, buf, len);
    return (int)recv(c->sock, buf, len, 0);
}

static void conn_close(Conn *c) {
    if (c->ssl) { SSL_shutdown(c->ssl); SSL_free(c->ssl); }
    if (c->ctx) SSL_CTX_free(c->ctx);
    if (c->sock >= 0) close(c->sock);
}

static int parse_http_meta(char *buf, HttpMeta *meta) {
    memset(meta, 0, sizeof(*meta));
    char *header_end = strstr(buf, "\r\n\r\n");
    if (!header_end) return 0;
    meta->header_len = (int)((header_end + 4) - buf);

    if (sscanf(buf, "HTTP/%*d.%*d %d", &meta->status_code) != 1) {
        meta->status_code = 0;
    }

    const char *needle = "\r\nLocation:";
    char *loc = strstr(buf, needle);
    if (!loc && strncmp(buf, "Location:", 9) == 0) loc = buf;
    if (loc) {
        loc += (loc == buf) ? 9 : (int)strlen(needle);
        while (*loc == ' ' || *loc == '\t') loc++;
        int i = 0;
        while (*loc && *loc != '\r' && *loc != '\n' && i < (int)sizeof(meta->location) - 1) {
            meta->location[i++] = *loc++;
        }
        meta->location[i] = '\0';
    }
    return 1;
}

static int parse_redirect_url(const char *location,
                              char *host_out, int host_len,
                              char *port_out, int port_len,
                              char *path_out, int path_len,
                              int *use_tls_out) {
    if (!location || !*location) return 0;

    if (strncmp(location, "https://", 8) == 0 || strncmp(location, "http://", 7) == 0) {
        const int use_tls = (strncmp(location, "https://", 8) == 0);
        const char *url = location + (use_tls ? 8 : 7);
        const char *slash = strchr(url, '/');
        const char *host_end = slash ? slash : url + strlen(url);
        const char *colon = NULL;
        for (const char *p = url; p < host_end; ++p) {
            if (*p == ':') colon = p;
        }

        int host_chars = (int)((colon ? colon : host_end) - url);
        if (host_chars <= 0 || host_chars >= host_len) return 0;
        memcpy(host_out, url, host_chars);
        host_out[host_chars] = '\0';

        if (colon) {
            int port_chars = (int)(host_end - colon - 1);
            if (port_chars <= 0 || port_chars >= port_len) return 0;
            memcpy(port_out, colon + 1, port_chars);
            port_out[port_chars] = '\0';
        } else {
            strncpy(port_out, use_tls ? "443" : "80", port_len - 1);
            port_out[port_len - 1] = '\0';
        }

        if (slash) {
            strncpy(path_out, slash, path_len - 1);
            path_out[path_len - 1] = '\0';
        } else {
            strncpy(path_out, "/", path_len - 1);
            path_out[path_len - 1] = '\0';
        }

        *use_tls_out = use_tls;
        return 1;
    }

    if (location[0] == '/') {
        strncpy(path_out, location, path_len - 1);
        path_out[path_len - 1] = '\0';
        return 2; /* relative redirect */
    }

    return 0;
}

static int http_request(const char *method,
                        const char *host, const char *port, int use_tls,
                        const char *path, const char *body,
                        char *buf, int buf_len, HttpMeta *meta) {
    Conn c;
    if (!conn_open(host, port, use_tls, &c)) return 0;

    char req[1024];
    int rlen;
    if (body) {
        rlen = snprintf(req, sizeof(req),
            "%s %s HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\n"
            "Content-Length: %zu\r\nConnection: close\r\n\r\n",
            method, path, host, strlen(body));
    } else {
        rlen = snprintf(req, sizeof(req),
            "%s %s HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n",
            method, path, host);
    }

    if (conn_send(&c, req, rlen) < 0) {
        conn_close(&c);
        return 0;
    }
    if (body && conn_send(&c, body, (int)strlen(body)) < 0) {
        conn_close(&c);
        return 0;
    }

    int total = 0, n;
    char tmp[4096];
    while ((n = conn_recv(&c, tmp, sizeof(tmp))) > 0) {
        if (total + n < buf_len) { memcpy(buf + total, tmp, n); total += n; }
    }
    conn_close(&c);
    buf[total] = '\0';

    return parse_http_meta(buf, meta);
}

static int http_get(const char *host, const char *port, int use_tls,
                    const char *path, char *buf, int buf_len) {
    char host_cur[128], port_cur[16], path_cur[512];
    strncpy(host_cur, host, sizeof(host_cur) - 1); host_cur[sizeof(host_cur) - 1] = '\0';
    strncpy(port_cur, port, sizeof(port_cur) - 1); port_cur[sizeof(port_cur) - 1] = '\0';
    strncpy(path_cur, path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
    int tls_cur = use_tls;

    for (int redirects = 0; redirects < 5; redirects++) {
        HttpMeta meta;
        if (!http_request("GET", host_cur, port_cur, tls_cur, path_cur, NULL, buf, buf_len, &meta)) {
            return 0;
        }
        if (meta.status_code >= 300 && meta.status_code < 400 && meta.location[0] != '\0') {
            char next_host[128], next_port[16], next_path[512];
            int next_tls = tls_cur;
            int rc = parse_redirect_url(meta.location,
                                        next_host, sizeof(next_host),
                                        next_port, sizeof(next_port),
                                        next_path, sizeof(next_path),
                                        &next_tls);
            if (rc == 1) {
                strncpy(host_cur, next_host, sizeof(host_cur) - 1); host_cur[sizeof(host_cur) - 1] = '\0';
                strncpy(port_cur, next_port, sizeof(port_cur) - 1); port_cur[sizeof(port_cur) - 1] = '\0';
                strncpy(path_cur, next_path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
                tls_cur = next_tls;
                continue;
            }
            if (rc == 2) {
                strncpy(path_cur, next_path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
                continue;
            }
            return 0;
        }
        char *body = strstr(buf, "\r\n\r\n");
        if (!body) return 0;
        body += 4;
        memmove(buf, body, strlen(body) + 1);
        return buf[0] == '{' ? 1 : 0;
    }
    return 0;
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

static const char *skip_ws_colon(const char *p) {
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n' || *p == ':') p++;
    return p;
}

static const char *find_json_key(const char *json, const char *key) {
    char search[128];
    snprintf(search, sizeof(search), "\"%s\"", key);
    return strstr(json, search);
}

static int json_object_span(const char *start, const char **end_out) {
    if (!start || *start != '{') return 0;
    int depth = 0;
    int in_str = 0;
    int escaped = 0;
    const char *p = start;
    while (*p) {
        char c = *p;
        if (in_str) {
            if (escaped) escaped = 0;
            else if (c == '\\') escaped = 1;
            else if (c == '"') in_str = 0;
        } else {
            if (c == '"') in_str = 1;
            else if (c == '{') depth++;
            else if (c == '}') {
                depth--;
                if (depth == 0) {
                    *end_out = p + 1;
                    return 1;
                }
            }
        }
        p++;
    }
    return 0;
}

static int extract_template_object(const char *json, char *out, int out_len) {
    const char *tmpl_key = find_json_key(json, "template");
    if (!tmpl_key) return 0;
    const char *start = skip_ws_colon(tmpl_key + strlen("\"template\""));
    if (*start != '{') return 0;

    const char *end = NULL;
    if (!json_object_span(start, &end)) return 0;

    int len = (int)(end - start);
    if (len <= 0 || len >= out_len) return 0;
    memcpy(out, start, len);
    out[len] = '\0';
    return 1;
}

static int replace_header_nonce(const char *block_json, uint64_t nonce,
                                char *out, int out_len) {
    const char *header_key = find_json_key(block_json, "header");
    if (!header_key) return 0;
    const char *header_start = skip_ws_colon(header_key + strlen("\"header\""));
    if (*header_start != '{') return 0;

    const char *header_end = NULL;
    if (!json_object_span(header_start, &header_end)) return 0;

    char header_buf[8192];
    int header_len = (int)(header_end - header_start);
    if (header_len <= 0 || header_len >= (int)sizeof(header_buf)) return 0;
    memcpy(header_buf, header_start, header_len);
    header_buf[header_len] = '\0';

    const char *nonce_key = find_json_key(header_buf, "nonce");
    if (!nonce_key) return 0;
    const char *value_start = skip_ws_colon(nonce_key + strlen("\"nonce\""));
    const char *value_end = value_start;
    if (*value_end == '-') value_end++;
    while (*value_end >= '0' && *value_end <= '9') value_end++;

    /* Resume from the same offset in the ORIGINAL block JSON — value_end
       points into the header_buf copy, which ends at the header's closing
       brace; printing from it would drop everything after the header
       (notably "transactions") and the node would reject the block. */
    int prefix = (int)(header_start - block_json) + (int)(value_start - header_buf);
    const char *tail = block_json
                     + (header_start - block_json)
                     + (value_end - header_buf);
    int written = snprintf(out, out_len, "%.*s%llu%s",
                           prefix, block_json,
                           (unsigned long long)nonce,
                           tail);
    return written > 0 && written < out_len;
}

static int http_post_json(const char *host, const char *port, int use_tls,
                          const char *path, const char *body,
                          char *buf, int buf_len) {
    char host_cur[128], port_cur[16], path_cur[512];
    strncpy(host_cur, host, sizeof(host_cur) - 1); host_cur[sizeof(host_cur) - 1] = '\0';
    strncpy(port_cur, port, sizeof(port_cur) - 1); port_cur[sizeof(port_cur) - 1] = '\0';
    strncpy(path_cur, path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
    int tls_cur = use_tls;

    for (int redirects = 0; redirects < 5; redirects++) {
        HttpMeta meta;
        if (!http_request("POST", host_cur, port_cur, tls_cur, path_cur, body, buf, buf_len, &meta)) {
            return 0;
        }
        if (meta.status_code >= 300 && meta.status_code < 400 && meta.location[0] != '\0') {
            char next_host[128], next_port[16], next_path[512];
            int next_tls = tls_cur;
            int rc = parse_redirect_url(meta.location,
                                        next_host, sizeof(next_host),
                                        next_port, sizeof(next_port),
                                        next_path, sizeof(next_path),
                                        &next_tls);
            if (rc == 1) {
                strncpy(host_cur, next_host, sizeof(host_cur) - 1); host_cur[sizeof(host_cur) - 1] = '\0';
                strncpy(port_cur, next_port, sizeof(port_cur) - 1); port_cur[sizeof(port_cur) - 1] = '\0';
                strncpy(path_cur, next_path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
                tls_cur = next_tls;
                continue;
            }
            if (rc == 2) {
                strncpy(path_cur, next_path, sizeof(path_cur) - 1); path_cur[sizeof(path_cur) - 1] = '\0';
                continue;
            }
            return 0;
        }

        char *body_start = strstr(buf, "\r\n\r\n");
        if (!body_start) return 0;
        body_start += 4;
        memmove(buf, body_start, strlen(body_start) + 1);
        return 1;
    }
    return 0;
}

// ── Template fetch ─────────────────────────────────────────────────────────────

/* Global buf so stack doesn't overflow (template_json is 1 MB) */
static char s_rpc_buf[RPC_BUF];
/* Stores raw template JSON: {"header":{...},"transactions":[...]} for submitblock */
static char s_template_json[RPC_BUF];

static int fetch_template(const char *host, const char *port, int use_tls,
                           const char *wallet, JobDesc *job) {
    char path[256];
    snprintf(path, sizeof(path), "/getblocktemplate/%s", wallet);
    if (!http_get(host, port, use_tls, path, s_rpc_buf, sizeof(s_rpc_buf))) return 0;

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

    /* epoch_seed lives at the response root, not inside template.header */
    if (!extract_byte_array(s_rpc_buf, "epoch_seed", job->epoch_seed, 32)) {
        fprintf(stderr,
            "[solo] template has no epoch_seed — tensorium-node is too old "
            "for TensorHash v1, upgrade the node\n");
        return 0;
    }

    snprintf(job->job_id, JOB_ID_LEN, "solo-%llu",
             (unsigned long long)job->height);
    job->valid = 1;

    /* Extract and cache raw template JSON for submitblock.
       The response has: {...,"template":{...},...}
       We need the value of "template" — the full Block JSON. */
    s_template_json[0] = '\0';
    extract_template_object(s_rpc_buf, s_template_json, sizeof(s_template_json));

    return 1;
}

// ── Block submit ───────────────────────────────────────────────────────────────

static int submit_block(const char *host, const char *port, int use_tls,
                        const JobDesc *job, uint64_t nonce) {
    /* Submit the cached template JSON with nonce replaced.
       s_template_json = {"header":{...,"nonce":0,...},"transactions":[...]}
       /submitblock expects this exact Block format. */
    if (s_template_json[0] == '\0') {
        fprintf(stderr, "[solo] no cached template for submitblock\n");
        return 0;
    }

    static char submit_json[RPC_BUF];
    if (!replace_header_nonce(s_template_json, nonce, submit_json, sizeof(submit_json))) {
        fprintf(stderr, "[solo] failed to update nonce in template\n");
        return 0;
    }

    char resp[1024] = {0};
    if (!http_post_json(host, port, use_tls, "/submitblock", submit_json, resp, sizeof(resp))) {
        fprintf(stderr, "[solo] submitblock HTTP request failed\n");
        return 0;
    }
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
    int use_tls = cfg->rpc_use_tls;

    JobDesc job;
    memset(&job, 0, sizeof(job));
    /* Solo mode: mine at full network difficulty — no pool, so share_bits = difficulty_bits */
    job.share_bits = 0; /* will be set from difficulty_bits after fetch */

    /* Fetch first template with retries */
    while (state->running && !fetch_template(host, port, use_tls, cfg->wallet, &job)) {
        fprintf(stderr, "[solo] failed to get template — retrying in 3s\n");
        fflush(stderr);
        sleep(3);
    }
    if (!state->running) return;

    /* In solo mode the kernel mines at full network difficulty.
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
            if (strcmp(share.job_id, job.job_id) != 0) {
                /* Stale share mined against a previous job — its nonce does
                   not satisfy the current template's header, so submitting
                   it would be rejected as invalid proof-of-work. */
                continue;
            }
            if (share.is_block) {
                printf("[solo] ⛏ BLOCK FOUND! height=%llu  nonce=%llu  GPU=%d\n",
                       (unsigned long long)job.height,
                       (unsigned long long)share.nonce, share.gpu_id);
                fflush(stdout);
                if (submit_block(host, port, use_tls, &job, share.nonce))
                    printf("[solo] block submitted OK — fetching new template\n");
                else
                    fprintf(stderr, "[solo] block submission failed\n");
                fflush(stdout);

                /* Force immediate template refresh so GPU stops re-mining same nonce */
                JobDesc fresh;
                memset(&fresh, 0, sizeof(fresh));
                for (int r = 0; r < 5 && state->running; r++) {
                    if (fetch_template(host, port, use_tls, cfg->wallet, &fresh)) {
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
            /* fetch_template overwrites the cached template JSON. If the
               refresh does NOT republish the job (same height/prev), the GPU
               keeps mining the OLD header (old timestamp) while submits
               would use the NEW template — every nonce would be rejected as
               invalid proof-of-work. Restore the cached template in that
               case so job and template stay in lockstep. */
            static char tmpl_backup[RPC_BUF];
            memcpy(tmpl_backup, s_template_json, sizeof(tmpl_backup));
            if (fetch_template(host, port, use_tls, cfg->wallet, &fresh)) {
                if (fresh.height != last_height ||
                    memcmp(fresh.previous_hash, job.previous_hash, 32) != 0) {
                    fresh.share_bits = fresh.difficulty_bits;
                    job = fresh;
                    last_height = job.height;
                    job_publish(state, &job);
                    printf("[solo] new block  height=%llu\n",
                           (unsigned long long)job.height);
                    fflush(stdout);
                } else {
                    memcpy(s_template_json, tmpl_backup, sizeof(tmpl_backup));
                }
            } else {
                memcpy(s_template_json, tmpl_backup, sizeof(tmpl_backup));
            }
            last_refresh = time(NULL);
        }
    }
}

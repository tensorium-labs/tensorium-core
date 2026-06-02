#!/bin/bash
# tensorium-monitor.sh — Phase 11 enhanced
# Canonical metadata: CANONICAL_ASSET_METADATA.md in tensorium-core repo

LOG="/var/log/tensorium-monitor.log"
STATE_FILE="/var/lib/tensorium-monitor/state.json"
ALERT_LOG="/var/log/tensorium-alerts.log"
STATE_WARN_MB=500
STATE_CRIT_MB=1024
STATUS=0

log()   { echo "$(date '+%Y-%m-%d %H:%M:%S') $1" >> "$LOG"; }
alert() { echo "$(date '+%Y-%m-%d %H:%M:%S') ALERT $1" | tee -a "$ALERT_LOG" >> "$LOG"; STATUS=1; }

mkdir -p /var/lib/tensorium-monitor

# Load previous MC height for stall detection
PREV_MC_HEIGHT=0
if [ -f "$STATE_FILE" ]; then
    PREV_MC_HEIGHT=$(python3 -c "import json; print(json.load(open('$STATE_FILE')).get('mc_height',0))" 2>/dev/null || echo 0)
fi

# ── Testnet RPC ───────────────────────────────────────────────────────────────
RPC=$(curl -sf --max-time 10 http://127.0.0.1:23332/health 2>/dev/null)
if echo "$RPC" | grep -q '"ok"'; then
    TN_HEIGHT=$(curl -sf --max-time 10 http://127.0.0.1:23332/getblockcount 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('height',0))" 2>/dev/null || echo 0)
    log "INFO testnet_rpc=ok height=${TN_HEIGHT}"
else
    alert "testnet_rpc=FAIL response='$RPC'"
fi

if ss -tlnp 2>/dev/null | grep -q ':23333'; then
    log "INFO testnet_p2p=ok"
else
    alert "testnet_p2p=FAIL port 23333 not listening"
fi

# ── Mainnet Candidate node ────────────────────────────────────────────────────
MC_RPC=$(curl -sf --max-time 10 http://127.0.0.1:33332/health 2>/dev/null)
if echo "$MC_RPC" | grep -q '"ok"'; then
    MC_HEIGHT=$(curl -sf --max-time 10 http://127.0.0.1:33332/getblockcount 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('height',0))" 2>/dev/null || echo 0)
    log "INFO mc_rpc=ok mc_height=${MC_HEIGHT}"
    if [ "$MC_HEIGHT" -le "$PREV_MC_HEIGHT" ] && [ "$PREV_MC_HEIGHT" -gt 0 ]; then
        log "WARN mc_stall=height_not_advancing prev=${PREV_MC_HEIGHT} current=${MC_HEIGHT}"
    fi
else
    alert "mc_rpc=FAIL response='$MC_RPC'"
    MC_HEIGHT=$PREV_MC_HEIGHT
fi

if ss -tlnp 2>/dev/null | grep -q ':33333'; then
    log "INFO mc_p2p=ok"
else
    alert "mc_p2p=FAIL port 33333 not listening"
fi

# ── State file / RocksDB disk size ────────────────────────────────────────────
for STATE_PATH in /root/node1/state.json /root/node1/tensorium-testnet-state.db \
                  /root/mc/tensorium-mc-state.json /root/mc/tensorium-mc-state.db; do
    if [ -e "$STATE_PATH" ]; then
        SIZE_MB=$(du -sm "$STATE_PATH" 2>/dev/null | cut -f1)
        SIZE_MB=${SIZE_MB:-0}
        log "INFO state_size path=${STATE_PATH} size=${SIZE_MB}MB"
        if [ "$SIZE_MB" -ge "$STATE_CRIT_MB" ]; then
            alert "state_critical size=${SIZE_MB}MB >= ${STATE_CRIT_MB}MB — ${STATE_PATH}"
        elif [ "$SIZE_MB" -ge "$STATE_WARN_MB" ]; then
            log "WARN state_growing size=${SIZE_MB}MB >= ${STATE_WARN_MB}MB — ${STATE_PATH}"
        fi
    fi
done

# ── Explorer + indexer ────────────────────────────────────────────────────────
EXP=$(curl -sf --max-time 5 http://127.0.0.1:3000/ 2>/dev/null)
if [ -n "$EXP" ]; then
    IDX=$(curl -sf --max-time 5 http://127.0.0.1:3000/api/indexer/status 2>/dev/null)
    if echo "$IDX" | python3 -c "import json,sys; exit(0 if json.load(sys.stdin).get('ready') else 1)" 2>/dev/null; then
        IDX_HEIGHT=$(echo "$IDX" | python3 -c "import json,sys; print(json.load(sys.stdin).get('lastHeight',0))" 2>/dev/null || echo 0)
        IDX_ADDRS=$(echo "$IDX" | python3 -c "import json,sys; print(json.load(sys.stdin).get('addresses',0))" 2>/dev/null || echo 0)
        log "INFO explorer=ok indexer_ready=true height=${IDX_HEIGHT} addresses=${IDX_ADDRS}"
    else
        log "WARN explorer=ok indexer_not_ready"
    fi
else
    alert "explorer=FAIL no response on port 3000"
fi

# ── Pool ──────────────────────────────────────────────────────────────────────
POOL=$(curl -sf --max-time 5 http://127.0.0.1:23336/health 2>/dev/null)
if echo "$POOL" | grep -q '"ok"'; then log "INFO pool=ok"; else alert "pool=FAIL"; fi

# ── Bridge relayer ────────────────────────────────────────────────────────────
if pm2 list 2>/dev/null | grep -q "tensorium-bridge-relayer.*online"; then
    log "INFO bridge_relayer=ok"
else
    alert "bridge_relayer=FAIL process not online"
fi

# ── Discord bot ───────────────────────────────────────────────────────────────
if systemctl is-active --quiet txm-discord-bot 2>/dev/null; then
    log "INFO discord_bot=ok"
else
    log "WARN discord_bot=not_active"
fi

# ── Public RPC ────────────────────────────────────────────────────────────────
_check_pub() { local name=$1 url=$2
    RESP=$(curl -sf --max-time 10 "$url" 2>/dev/null)
    if [ -n "$RESP" ]; then log "INFO ${name}=ok"; else alert "${name}=FAIL"; fi
}
_check_pub pub_rpc        https://rpc.tensoriumlabs.com/health
_check_pub mc_pub_rpc     https://mc-rpc.tensoriumlabs.com/health
_check_pub faucet         https://faucet.tensoriumlabs.com/health

# ── Disk ──────────────────────────────────────────────────────────────────────
DISK=$(df / --output=pcent 2>/dev/null | tail -1 | tr -d ' %')
if [ -n "$DISK" ] && [ "$DISK" -gt 85 ]; then alert "disk=${DISK}% (>85%)";
elif [ -n "$DISK" ] && [ "$DISK" -gt 75 ]; then log "WARN disk=${DISK}%";
else log "INFO disk=${DISK}%"; fi

# ── SSL ───────────────────────────────────────────────────────────────────────
EXPIRY=$(echo | openssl s_client -servername tensoriumlabs.com \
    -connect tensoriumlabs.com:443 2>/dev/null \
    | openssl x509 -noout -enddate 2>/dev/null | cut -d= -f2)
if [ -n "$EXPIRY" ]; then
    DAYS=$(( ( $(date -d "$EXPIRY" +%s) - $(date +%s) ) / 86400 ))
    if [ "$DAYS" -lt 14 ]; then alert "ssl_expiry=${DAYS}days";
    elif [ "$DAYS" -lt 30 ]; then log "WARN ssl_expiry=${DAYS}days";
    else log "INFO ssl_expiry=${DAYS}days"; fi
fi

# ── Persist state ─────────────────────────────────────────────────────────────
python3 -c "
import json
state = {}
try:
    with open('$STATE_FILE') as f: state = json.load(f)
except: pass
state['mc_height'] = $MC_HEIGHT
state['last_check'] = '$(date -u +%Y-%m-%dT%H:%M:%SZ)'
with open('$STATE_FILE', 'w') as f: json.dump(state, f)
" 2>/dev/null

if [ "$STATUS" -ne 0 ]; then log "STATUS: DEGRADED"; else log "STATUS: OK"; fi

#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Tensorium Mainnet Installer
# Usage:  curl -fsSL https://raw.githubusercontent.com/tensorium-labs/tensorium-core/main/install.sh | bash
# ---------------------------------------------------------------------------

REPO="tensorium-labs/tensorium-core"
VERSION="v0.4.0-mainnet"
SEED_NODE="seed.tensoriumlabs.com"
RPC_PORT="33332"
P2P_PORT="33333"
CHAIN_ID="tensorium-mainnet"
INSTALL_DIR="/usr/local/bin"
DATA_DIR="$HOME/tensorium-mainnet-node"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()    { echo -e "${CYAN}[tensorium]${NC} $*"; }
success() { echo -e "${GREEN}[tensorium]${NC} $*"; }
warn()    { echo -e "${YELLOW}[tensorium]${NC} $*"; }
fatal()   { echo -e "${RED}[tensorium] ERROR:${NC} $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64)  echo "x86_64" ;;
        aarch64) echo "aarch64" ;;
        *)        fatal "Unsupported architecture: $arch (only x86_64 and aarch64 are supported)" ;;
    esac
}

detect_os() {
    case "$(uname -s)" in
        Linux)  echo "linux" ;;
        Darwin) echo "darwin" ;;
        *)       fatal "Unsupported OS: $(uname -s) (Linux and macOS only)" ;;
    esac
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || fatal "Required command not found: $1 — install it first"
}

check_root_or_sudo() {
    if [[ $EUID -ne 0 ]]; then
        if command -v sudo >/dev/null 2>&1; then
            SUDO="sudo"
        else
            fatal "This installer needs root or sudo to install binaries to $INSTALL_DIR"
        fi
    else
        SUDO=""
    fi
}

# ---------------------------------------------------------------------------
# Download
# ---------------------------------------------------------------------------

download_binaries() {
    local os arch base_url
    os="$(detect_os)"
    arch="$(detect_arch)"
    base_url="https://github.com/${REPO}/releases/download/${VERSION}"

    info "Downloading Tensorium ${VERSION} for ${os}-${arch}..."

    # txmminer (CPU) is dev-only and not distributed to end users
    local bins=("tensorium-node" "txmwallet")
    for bin in "${bins[@]}"; do
        local filename="${bin}-${os}-${arch}"
        local url="${base_url}/${filename}"
        local dest="/tmp/${filename}"

        info "  Downloading ${filename}..."
        if command -v curl >/dev/null 2>&1; then
            curl -fsSL -o "$dest" "$url" || fatal "Failed to download $url"
        elif command -v wget >/dev/null 2>&1; then
            wget -q -O "$dest" "$url" || fatal "Failed to download $url"
        else
            fatal "Neither curl nor wget found — install one first"
        fi

        chmod +x "$dest"
        $SUDO mv "$dest" "${INSTALL_DIR}/${bin}"
        success "  Installed ${bin} → ${INSTALL_DIR}/${bin}"
    done
}

# ---------------------------------------------------------------------------
# Wallet setup
# ---------------------------------------------------------------------------

setup_wallet() {
    echo ""
    info "Setting up wallet..."
    mkdir -p "$DATA_DIR"

    if [[ -f "$DATA_DIR/wallet.json" ]]; then
        warn "Wallet already exists at $DATA_DIR/wallet.json — skipping"
        return
    fi

    echo ""
    echo -e "${BOLD}Enter a passphrase to encrypt your wallet (remember it!):${NC}"
    read -r -s WALLET_PASS
    echo ""
    echo -e "${BOLD}Confirm passphrase:${NC}"
    read -r -s WALLET_PASS2
    echo ""

    if [[ "$WALLET_PASS" != "$WALLET_PASS2" ]]; then
        fatal "Passphrases do not match"
    fi

    if [[ -z "$WALLET_PASS" ]]; then
        fatal "Passphrase cannot be empty"
    fi

    TENSORIUM_WALLET="$DATA_DIR/wallet.json" \
    TENSORIUM_WALLET_PASSPHRASE="$WALLET_PASS" \
        txmwallet create

    local address
    address=$(TENSORIUM_WALLET="$DATA_DIR/wallet.json" \
              TENSORIUM_WALLET_PASSPHRASE="$WALLET_PASS" \
              txmwallet getnewaddress)

    echo ""
    success "Wallet created!"
    echo -e "  ${BOLD}Address:${NC} ${GREEN}${address}${NC}"
    echo "$address" > "$DATA_DIR/miner.address"
    echo ""
    warn "IMPORTANT: Back up $DATA_DIR/wallet.json and remember your passphrase."
    warn "           If you lose either, your funds are gone."
    MINER_ADDRESS="$address"
    WALLET_PASSPHRASE="$WALLET_PASS"
}

# ---------------------------------------------------------------------------
# Node setup
# ---------------------------------------------------------------------------

setup_node() {
    echo ""
    info "Setting up node data directory at $DATA_DIR..."
    mkdir -p "$DATA_DIR"

    local state="$DATA_DIR/state.json"
    local state_db="$DATA_DIR/state.db"
    local mempool="$DATA_DIR/mempool.json"
    local bans="$DATA_DIR/banlist.json"

    if [[ -f "$state" || -d "$state_db" ]]; then
        warn "Existing chain state detected ($state or $state_db) — skipping init"
    else
        info "Initializing mainnet chain (genesis block)..."
        TENSORIUM_STATE="$state" \
        TENSORIUM_MEMPOOL="$mempool" \
        TENSORIUM_BANS="$bans" \
            tensorium-node init

        info "Syncing from seed node ${SEED_NODE}..."
        TENSORIUM_STATE="$state" \
        TENSORIUM_MEMPOOL="$mempool" \
        TENSORIUM_BANS="$bans" \
        TENSORIUM_PEERS="${SEED_NODE}:${P2P_PORT}" \
            tensorium-node sync "${SEED_NODE}:${P2P_PORT}" || warn "Sync failed — you can run 'tensorium-node sync ${SEED_NODE}:${P2P_PORT}' manually later"
    fi
}

# ---------------------------------------------------------------------------
# Systemd service (optional, Linux only)
# ---------------------------------------------------------------------------

install_service() {
    if [[ "$(detect_os)" != "linux" ]]; then
        return
    fi
    if ! command -v systemctl >/dev/null 2>&1; then
        return
    fi

    echo ""
    echo -e "${BOLD}Install systemd service so the node starts automatically on boot? [y/N]${NC}"
    read -r -n1 INSTALL_SVC
    echo ""
    [[ "$INSTALL_SVC" =~ ^[Yy]$ ]] || return

    local state="$DATA_DIR/state.json"
    local mempool="$DATA_DIR/mempool.json"
    local bans="$DATA_DIR/banlist.json"
    local user="${SUDO_USER:-$(whoami)}"

    $SUDO tee /etc/systemd/system/tensorium-rpc.service > /dev/null <<EOF
[Unit]
Description=Tensorium Mainnet RPC
After=network.target

[Service]
Type=simple
User=${user}
WorkingDirectory=${DATA_DIR}
Environment=TENSORIUM_STATE=${state}
Environment=TENSORIUM_MEMPOOL=${mempool}
Environment=TENSORIUM_BANS=${bans}
Environment=TENSORIUM_PEERS=${SEED_NODE}:${P2P_PORT}
ExecStart=${INSTALL_DIR}/tensorium-node rpc 127.0.0.1:${RPC_PORT}
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

    $SUDO tee /etc/systemd/system/tensorium-p2p.service > /dev/null <<EOF
[Unit]
Description=Tensorium Mainnet P2P
After=network.target

[Service]
Type=simple
User=${user}
WorkingDirectory=${DATA_DIR}
Environment=TENSORIUM_STATE=${state}
Environment=TENSORIUM_MEMPOOL=${mempool}
Environment=TENSORIUM_BANS=${bans}
ExecStart=${INSTALL_DIR}/tensorium-node p2p-listen 0.0.0.0:${P2P_PORT}
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

    $SUDO systemctl daemon-reload
    $SUDO systemctl enable tensorium-rpc tensorium-p2p
    $SUDO systemctl start tensorium-rpc tensorium-p2p
    success "Systemd services installed and started"
    info "  Check status:  sudo systemctl status tensorium-rpc tensorium-p2p"
    info "  View logs:     sudo journalctl -u tensorium-rpc -f"
}

# ---------------------------------------------------------------------------
# Print summary
# ---------------------------------------------------------------------------

print_summary() {
    local miner_addr="${MINER_ADDRESS:-<run txmwallet getnewaddress>}"

    echo ""
    echo -e "${GREEN}${BOLD}══════════════════════════════════════════════${NC}"
    echo -e "${GREEN}${BOLD}  Tensorium ${VERSION} installed successfully!${NC}"
    echo -e "${GREEN}${BOLD}══════════════════════════════════════════════${NC}"
    echo ""
    echo -e "  ${BOLD}Network:${NC}      ${CHAIN_ID}"
    echo -e "  ${BOLD}Seed node:${NC}    ${SEED_NODE}:${P2P_PORT}"
    echo -e "  ${BOLD}Data dir:${NC}     ${DATA_DIR}"
    echo -e "  ${BOLD}Miner addr:${NC}   ${miner_addr}"
    echo ""
    echo -e "${BOLD}Useful commands:${NC}"
    echo ""
    echo -e "  Start node (manual):"
    echo -e "    ${CYAN}TENSORIUM_STATE=${DATA_DIR}/state.json TENSORIUM_MEMPOOL=${DATA_DIR}/mempool.json TENSORIUM_BANS=${DATA_DIR}/banlist.json tensorium-node rpc 127.0.0.1:${RPC_PORT} &${NC}"
    echo -e "    ${CYAN}TENSORIUM_STATE=${DATA_DIR}/state.json TENSORIUM_MEMPOOL=${DATA_DIR}/mempool.json TENSORIUM_BANS=${DATA_DIR}/banlist.json tensorium-node p2p-listen 0.0.0.0:${P2P_PORT} &${NC}"
    echo -e "    ${CYAN}# put nginx in front before exposing RPC publicly${NC}"
    echo ""
    echo -e "  Start mining (GPU — NVIDIA RTX 3000/4000/5000 required):"
    echo -e "    ${CYAN}# Download tensorium-miner from Releases${NC}"
    echo -e "    ${CYAN}# https://github.com/${REPO}/releases/latest${NC}"
    echo -e "    ${CYAN}chmod +x tensorium-miner && sudo mv tensorium-miner /usr/local/bin/${NC}"
    echo -e "    ${CYAN}# Solo mining (0% fee — full reward to your address):${NC}"
    echo -e "    ${CYAN}tensorium-miner --mode solo --rpc http://127.0.0.1:${RPC_PORT} --wallet ${miner_addr}${NC}"
    echo -e "    ${CYAN}# Pool mining (5% fee — smoothed payouts, no node required):${NC}"
    echo -e "    ${CYAN}tensorium-miner --mode pool --pool stratum+tcp://pooltxm.tensoriumlabs.com:23336 --wallet ${miner_addr}${NC}"
    echo ""
    echo -e "  Check chain height:"
    echo -e "    ${CYAN}curl -s http://localhost:${RPC_PORT}/getblockcount${NC}"
    echo ""
    echo -e "  Wallet balance:"
    echo -e "    ${CYAN}TENSORIUM_WALLET=${DATA_DIR}/wallet.json TENSORIUM_WALLET_PASSPHRASE=<pass> txmwallet balance${NC}"
    echo ""
    echo -e "  Chain state:"
    echo -e "    ${CYAN}${DATA_DIR}/state.json${NC} (compat path, auto-migrates to ${CYAN}${DATA_DIR}/state.db/${NC})"
    echo ""
    echo -e "  Docs & source: ${CYAN}https://github.com/${REPO}${NC}"
    echo ""
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    echo ""
    echo -e "${BOLD}Tensorium Mainnet Installer${NC}"
    echo -e "Version: ${VERSION} | Chain: ${CHAIN_ID}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    require_cmd uname
    check_root_or_sudo
    download_binaries
    setup_wallet
    setup_node
    install_service
    print_summary
}

main "$@"

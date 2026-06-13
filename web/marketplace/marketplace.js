// ── Pure helpers (unit-tested) ──────────────────────────────────────────────
export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

export const fmtAtoms = (a) =>
  `${Math.floor(Number(a) / 1e8)}.${(Number(a) % 1e8).toString().padStart(8, "0")}`;

// MUST match relay api-server.js listMsg/actMsg exactly.
export const listingMessage = (t) => `list:${t.asset_id_hex}:${t.amount}:${t.price_atoms}:${t.seller_addr}`;
export const acceptMessage = (id) => `accept:${id}`;
export const cancelMessage = (id) => `cancel:${id}`;

const shortId = (h) => (h ? `${h.slice(0, 8)}…${h.slice(-6)}` : "");

export function renderListingCard(l) {
  return `
    <div class="card asset-card">
      <div class="tick">${esc(l.ticker || "NFT")} <span class="tag">${l.kind === "nft" ? "NFT" : "TXM20"}</span></div>
      <div class="id">${esc(shortId(l.asset_id_hex))}</div>
      <div class="row"><span>Amount</span><span>${esc(String(l.amount))}</span></div>
      <div class="row"><span>Price</span><span>${fmtAtoms(l.price_atoms)} TXM</span></div>
      <div class="row"><span>Seller</span><span>${esc(shortId(l.seller_addr))}</span></div>
      <button class="wallet-btn buy-btn" data-listing="${esc(l.listing_id)}" data-price="${esc(String(l.price_atoms))}">Buy</button>
    </div>`;
}

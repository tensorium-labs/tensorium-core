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

// ── Wallet + relay flows (deps injected: { wallet, api }) ────────────────────
export async function connectWallet({ wallet }) {
  const accts = await wallet.requestAccounts();
  if (!accts || !accts.length) throw new Error("No account in wallet");
  return accts[0];
}

export async function createListing(form, { wallet, api }) {
  const seller_addr = await wallet.getAddress();
  const terms = { asset_id_hex: form.asset_id_hex, amount: Number(form.amount), price_atoms: Number(form.price_atoms), seller_addr, kind: form.kind };
  const { pubkey, sig } = await wallet.signMessage(listingMessage(terms));
  return api("/relay/listing", { terms, seller_pubkey: pubkey, sig });
}

export async function buyListing(listing_id, { wallet, api }) {
  const buyer_addr = await wallet.getAddress();
  const quote = await api("/relay/quote", { listing_id, buyer_addr });
  const signedTx = await wallet.signAssetTxPartial(quote.unsignedTx, quote.input_indices.buyer, quote.summary);
  return api("/relay/settlement", { listing_id, signedTx, buyer_addr });
}

export async function acceptSale(listing, { wallet, api }) {
  const fullySignedTx = await wallet.signAssetTxPartial(listing.settlement.signedTx, [0], { description: "Accept sale" });
  const { pubkey, sig } = await wallet.signMessage(acceptMessage(listing.listing_id));
  return api("/relay/accept", { listing_id: listing.listing_id, fullySignedTx, seller_pubkey: pubkey, sig });
}

export async function cancelListing(listing_id, { wallet, api }) {
  const { pubkey, sig } = await wallet.signMessage(cancelMessage(listing_id));
  return api("/relay/cancel", { listing_id, seller_pubkey: pubkey, sig });
}

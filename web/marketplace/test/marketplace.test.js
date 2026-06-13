import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { esc, fmtAtoms, listingMessage, acceptMessage, cancelMessage, renderListingCard } from "../marketplace.js";
import { connectWallet, createListing, buyListing, acceptSale, cancelListing } from "../marketplace.js";

describe("esc", () => {
  it("neutralizes HTML so a malicious ticker cannot inject script", () => {
    assert.equal(esc(`<img src=x onerror=alert(1)>`), "&lt;img src=x onerror=alert(1)&gt;");
    assert.equal(esc("GOLD"), "GOLD");
  });
});

describe("fmtAtoms", () => {
  it("formats atoms as TXM with 8 decimals", () => {
    assert.equal(fmtAtoms(123_45678900), "123.45678900");
    assert.equal(fmtAtoms(0), "0.00000000");
  });
});

describe("canonical messages (must match relay api-server.js)", () => {
  it("listingMessage", () => {
    assert.equal(listingMessage({ asset_id_hex: "aa", amount: 100, price_atoms: 5000000, seller_addr: "txm1s" }),
      "list:aa:100:5000000:txm1s");
  });
  it("acceptMessage / cancelMessage", () => {
    assert.equal(acceptMessage("lst_1"), "accept:lst_1");
    assert.equal(cancelMessage("lst_1"), "cancel:lst_1");
  });
});

describe("renderListingCard", () => {
  it("escapes user fields and includes price + a Buy button bound to the listing id", () => {
    const html = renderListingCard({ listing_id: "lst_x", asset_id_hex: "ab".repeat(32), kind: "txm20", amount: 100, price_atoms: 5000000, seller_addr: "txm1s", ticker: "<b>X</b>" });
    assert.ok(html.includes("&lt;b&gt;X&lt;/b&gt;"));        // ticker escaped
    assert.ok(!html.includes("<b>X</b>"));
    assert.ok(html.includes("data-listing=\"lst_x\""));      // buy button target
    assert.ok(html.includes("0.05000000"));                  // price in TXM
  });
});

const fakeWallet = (over = {}) => ({
  requestAccounts: async () => ["txm1seller00000000000000000000000000000000"],
  getAddress: async () => "txm1seller00000000000000000000000000000000",
  signMessage: async (m) => ({ pubkey: "03aa", sig: "30deadbeef", _msg: m }),
  signAssetTxPartial: async (tx, idx) => ({ ...tx, _signedIndices: idx }),
  ...over,
});
const fakeApi = (routes) => async (path, body) => {
  const r = routes[path.split("?")[0]];
  return typeof r === "function" ? r(body) : r;
};

describe("createListing", () => {
  it("signs the canonical listing message and POSTs terms+pubkey+sig", async () => {
    let posted;
    const api = fakeApi({ "/relay/listing": (b) => { posted = b; return { listing_id: "lst_1", state: "listed" }; } });
    const wallet = fakeWallet();
    const out = await createListing({ asset_id_hex: "aa", amount: 100, price_atoms: 5000000, kind: "txm20" }, { wallet, api });
    assert.equal(out.listing_id, "lst_1");
    assert.equal(posted.sig, "30deadbeef");
    assert.equal(posted.seller_pubkey, "03aa");
    assert.equal(posted.terms.seller_addr, "txm1seller00000000000000000000000000000000");
  });
});

describe("buyListing", () => {
  it("quotes, partial-signs the buyer inputs, and posts the settlement", async () => {
    let settle;
    const api = fakeApi({
      "/relay/quote": { listing_id: "lst_1", unsignedTx: { inputs: [{}, {}] }, summary: {}, input_indices: { seller: [0], buyer: [1] } },
      "/relay/settlement": (b) => { settle = b; return { state: "pending_settlement" }; },
    });
    const wallet = fakeWallet();
    const out = await buyListing("lst_1", { wallet, api });
    assert.equal(out.state, "pending_settlement");
    assert.deepEqual(settle.signedTx._signedIndices, [1]); // buyer inputs signed
    assert.equal(settle.listing_id, "lst_1");
  });
});

describe("acceptSale", () => {
  it("signs input[0], signs the accept message, and posts both", async () => {
    let acc;
    const api = fakeApi({ "/relay/accept": (b) => { acc = b; return { state: "broadcast", broadcast_txid: "beef" }; } });
    const wallet = fakeWallet();
    const out = await acceptSale({ listing_id: "lst_1", settlement: { signedTx: { inputs: [{}, {}] } } }, { wallet, api });
    assert.equal(out.broadcast_txid, "beef");
    assert.deepEqual(acc.fullySignedTx._signedIndices, [0]);
    assert.equal(acc.sig, "30deadbeef");
  });
});

describe("cancelListing", () => {
  it("signs cancel:<id> and posts", async () => {
    let c;
    const api = fakeApi({ "/relay/cancel": (b) => { c = b; return { cancelled: true }; } });
    const out = await cancelListing("lst_1", { wallet: fakeWallet(), api });
    assert.equal(out.cancelled, true);
    assert.equal(c.listing_id, "lst_1");
  });
});

import { royaltyPctToBps, createTokenFlow, createNftFlow } from "../marketplace.js";
describe("create-asset flows", () => {
  const wallet = { getAddress: async () => "txm1creator00000000000000000000000000000000", signAssetTx: async () => "deadbeefTXID" };
  describe("royaltyPctToBps", () => {
    it("converts percent to basis points, clamped to 0..10000", () => {
      assert.equal(royaltyPctToBps(5), 500);
      assert.equal(royaltyPctToBps(2.5), 250);
      assert.equal(royaltyPctToBps(200), 10000);
      assert.equal(royaltyPctToBps(-1), 0);
    });
  });
  it("createTokenFlow posts build-issue with creator addr and signs", async () => {
    let body; const api = async (p, b) => { body = b; return { unsignedTx: { inputs: [] }, summary: { action: "issue" } }; };
    const out = await createTokenFlow({ ticker: "GOLD", decimals: 8, supply: 1000, name: "Gold" }, { wallet, api });
    assert.equal(body.ticker, "GOLD"); assert.equal(body.creator_addr, "txm1creator00000000000000000000000000000000");
    assert.equal(out.asset_id, "deadbeefTXID");
  });
  it("createNftFlow converts royalty% to bps, defaults royalty addr to creator, signs", async () => {
    let body; const api = async (p, b) => { body = b; return { unsignedTx: {}, summary: { action: "mint" } }; };
    const out = await createNftFlow({ name: "Art", uri: "ipfs://x", content_hash: "ab".repeat(32), royalty_pct: 7.5 }, { wallet, api });
    assert.equal(body.royalty_bps, 750);
    assert.equal(body.royalty_addr, "txm1creator00000000000000000000000000000000");
    assert.equal(out.txid, "deadbeefTXID");
  });
});

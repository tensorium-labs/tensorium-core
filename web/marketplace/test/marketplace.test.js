import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { esc, fmtAtoms, listingMessage, acceptMessage, cancelMessage, renderListingCard } from "../marketplace.js";

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

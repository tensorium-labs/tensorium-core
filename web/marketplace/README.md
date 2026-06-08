# marketplace.tensoriumlabs.com — frontend

Static frontend for the Tensorium native-asset marketplace (Layer 5).
Served from `/var/www/marketplace` on the DO node; reuses the root site's
`assets/core.css` design system. Reads the asset indexer (Layer 2) via the
same-origin `/api/` nginx reverse proxy → `127.0.0.1:23340` (read-only, GET-only,
rate-limited).

Deploy: copy `index.html` to `/var/www/marketplace/index.html`; ensure
`assets/core.css` is present (copied from the root site). nginx site:
`marketplace.tensoriumlabs.com` (SSL via certbot), `location /api/` proxies the
indexer.

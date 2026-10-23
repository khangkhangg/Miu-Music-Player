# Landing page: musicplayer.vietnamcos.com

Static files (`index.html`, `lang.js`, `miu-cat.svg`, `shots/`, `i18n/`: English is in the page, the other ten languages load from `i18n/<code>.json`; `tests/js/site.test.js` checks they all have every key) served by nginx on **spo2x** from `/var/www/musicplayer`.
Cloudflare proxies the name (orange cloud), so nginx answers on 443 with the vietnamcos.com
certificate. Vhost: `nginx-musicplayer.vietnamcos.com.conf` here, installed as
`/etc/nginx/sites-available/musicplayer.vietnamcos.com` (symlinked into `sites-enabled`).
Read `C:\Projects\courses\docs\INFRASTRUCTURE.md` before touching the server.

Preview locally: `node tools/serve-site.cjs` → http://localhost:4173

Update the live page:

```bash
scp -i ~/.ssh/spo2x_admin -r site/index.html site/lang.js site/miu-cat.svg site/shots site/i18n root@100.71.6.80:/var/www/musicplayer/
```

No reload is needed for file changes. `index.html` is sent with `Cache-Control: no-cache`.

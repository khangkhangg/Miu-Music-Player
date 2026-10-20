// Serve site/ on http://localhost:4173 to preview the landing page. Usage: node tools/serve-site.cjs
const http = require('http');
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..', 'site');
const types = { '.html': 'text/html; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.css': 'text/css', '.js': 'text/javascript' };

http
  .createServer((req, res) => {
    const url = decodeURIComponent(req.url.split('?')[0]);
    const file = path.join(root, url.endsWith('/') ? url + 'index.html' : url);
    if (!file.startsWith(root)) return res.writeHead(403).end();
    fs.readFile(file, (err, data) => {
      if (err) return res.writeHead(404).end('not found');
      res.writeHead(200, { 'content-type': types[path.extname(file)] ?? 'application/octet-stream' }).end(data);
    });
  })
  .listen(4173, () => console.log('site on http://localhost:4173'));

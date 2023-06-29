const HARBINGER_PORT = HARBINGER_TMPL_PORT;
const HARBINGER_ORIGIN_HOST = "HARBINGER_TMPL_ORIGIN_HOST";

const INTERNAL_PATHS = [
    '/harbinger',
    '/harbinger_app.js',
    '/harbinger_worker.js',
];

self.addEventListener('activate', (event) => {
    console.log("ServiceWorker activate")
});

self.addEventListener('install', (event) => {
    console.log('ServiceWorker install')
});

function rewriteUrl(reqUrlStr) {
    const url = new URL(reqUrlStr, 'http://localhost');
    if (url.hostname === 'localhost') {
        if (INTERNAL_PATHS.includes(url.pathname)) {
            return url;
        }
        url.hostname = HARBINGER_ORIGIN_HOST;
    }
    if (url.hostname !== HARBINGER_ORIGIN_HOST) {
        url.pathname = `${url.hostname}${url.pathname}`;
    }
    url.hostname = 'localhost';
    url.port = HARBINGER_PORT;
    url.protocol = 'http';
    return url;
}

self.addEventListener('fetch', (event) => {
    event.respondWith((async () => {
        const reqUrl = rewriteUrl(event.request.url);
        if (reqUrl !== event.request.url) {
            console.log(`translating url: ${event.request.url} => ${reqUrl}`);
        } else {
            console.log(`leaving url unchanged: ${reqUrl}`)
        }
        const req = new Request(reqUrl, {
            ...event.request,
            duplex: 'half',
        });
        return await fetch(req);
    })());
});

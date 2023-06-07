const HARBINGER_PORT = HARBINGER_TMPL_PORT;

self.addEventListener('activate', (event) => {
    console.log("ServiceWorker activate")
});

self.addEventListener('install', (event) => {
    console.log('ServiceWorker install')
});

function normalizeUrl(urlStr) {
    try {
        return new URL(urlStr);
    } catch (e) {
        // relative URL, resolve based on which content path we're in
        const path = location.pathname;
        if (!path.startsWith('/content/')) {
            throw new Error(`unexpected relative URL ${location.pathname}`);
        }
        const hostname = path.split('/')[1];
        return new URL(urlStr, `https://${hostname}`);
    }
}

function rewriteUrl(urlStr) {
    const url = normalizeUrl(urlStr);
    if (url.hostname === 'localhost') {
        return url;
    }
    const newPathname = `srv/${url.hostname}${url.pathname}`;
    url.pathname = newPathname;
    url.hostname = 'localhost';
    url.port = HARBINGER_PORT;
    url.protocol = 'http';
    return url;
}

self.addEventListener('fetch', (event) => {
    const url = rewriteUrl(event.request.url);
    if (url !== event.request.url) {
        console.log(`translating url: ${event.request.url} => ${url}`);
    } else {
        console.log(`leaving url unchanged: ${url}`)
    }
    event.request.url = url;
    event.respondWith(fetch(event.request));
});

const HARBINGER_PORT = HARBINGER_TMPL_PORT;

self.addEventListener('activate', (event) => {
    console.log("ServiceWorker activate")
});

self.addEventListener('install', (event) => {
    console.log('ServiceWorker install')
});

function getVirtualHostname(clientUrl) {
    let parts = clientUrl.pathname.split('/');
    if (parts.length < 3 || parts[0] !== '' || parts[1] !== 'srv') {
        throw new Error(`can't get virtual hostname from url: ${clientUrl.toString()}`);
    }
    return parts[2];
}

function rewriteUrl(reqUrlStr, clientUrl) {
    const url = new URL(reqUrlStr, 'http://localhost');
    const vhost = getVirtualHostname(clientUrl);
    const newPathname = `srv/${vhost}${url.pathname}`;
    url.pathname = newPathname;
    url.hostname = 'localhost';
    url.port = HARBINGER_PORT;
    url.protocol = 'http';
    return url;
}

async function handleFetch(event) {
}

self.addEventListener('fetch', (event) => {
    if (!event.clientId) {
        console.log(`undefined client, ignoring request ${event.request.url}`)
        return;
    }
    event.respondWith(self.clients.get(event.clientId)
        .then(client => {
            let clientUrl = new URL(client.url);
            if (clientUrl.hostname === 'localhost' && clientUrl.pathname === '/') {
                return;
            }
            const reqUrl = rewriteUrl(event.request.url, clientUrl);
            if (reqUrl !== event.request.url) {
                console.log(`translating url: ${event.request.url} => ${reqUrl}`);
            } else {
                console.log(`leaving url unchanged: ${reqUrl}`)
            }
            const req = new Request(reqUrl, event.request);
            console.log(req)
            return fetch(req);
        }));
});

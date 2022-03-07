let transfers = {};

function dec2hex (dec) {
  return dec.toString(16).padStart(2, "0")
}

function generateId (len) {
  var arr = new Uint8Array((len || 40) / 2)
  crypto.getRandomValues(arr)
  return Array.from(arr, dec2hex).join('')
}

self.addEventListener('install', function(event) {
    event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', function(event) {
    event.waitUntil(self.clients.claim());
});

self.addEventListener('fetch', function(event) {
    console.log(event.request.url);

    let url = event.request.url;

    if (transfers[url] == undefined) {
        console.log("Responding normally");
        return null;
    }

    console.log("Redirecting response to piped JS bytes");
    let transfer = transfers[url];

    console.log("Responding");
    event.respondWith(transfer.response);
    console.log("Responded");
});

self.addEventListener('message', function(event) {
    console.log(event);

    let data = event.data;
    let port = event.ports[0];

    console.log(data);

    if (data.type === 'StartDownload') {
        let id = generateId(10);

        let url = `${self.registration.scope}${id}`;

        console.log(url);
        console.log("Creating stream");

        let stream = new ReadableStream(
            {
                start(controller) {
                    console.log("Starting");
                    port.onmessage = function(event) {
                        // console.log("Got message");
                        // console.log(event);

                        let { data, done } = event.data;

                        console.log("Received chunk, done: " + done);

                        if (done) {
                            console.log("Response complete, closing");
                            controller.close();
                            return;
                        }

                        controller.enqueue(data);
                    };

                    console.log("Finished starting");
                },
            });

        console.log("Created stream");

        transfers[url] = {
            headers: data.headers,
            response: new Response(stream, { headers: data.headers }),
        };

        console.log(`Added '${url}' to custom fetch`);

        port.postMessage(url);

        console.log(`Posted URL to port`);
    }
});

if ('serviceWorker' in navigator) {
    window.addEventListener('load', function() {
        navigator.serviceWorker
            .register('/sw.js')
            .then(
                function(reg) {
                    console.log('ServiceWorker registration successful with scope: ', reg.scope);
},
                function(err) {
                    console.log('ServiceWorker registration failed: ', err);
                });
    });
} else {
    console.error("No service worker!");
}

let firefox = navigator.userAgent.indexOf('Firefox') !== -1;
let href = window.location.href;
let host = window.location.host;
let secure = window.location.protocol.startsWith("https");
let ws = secure ? `wss://${host}/signalling` : `ws://localhost:8080/signalling`;
let msgChannel = new MessageChannel();

function dec2hex (dec) {
  return dec.toString(16).padStart(2, "0")
}

function generateId (len) {
  var arr = new Uint8Array((len || 40) / 2)
  window.crypto.getRandomValues(arr)
  return Array.from(arr, dec2hex).join('')
}


let chooseDiv = document.getElementById("choose");
let shareDiv = document.getElementById("share");
let sharingDiv = document.getElementById("sharing");
let statusDiv = document.getElementById("share-link");
let shareButton = document.getElementById("share-btn");
let dlBtn = document.getElementById("dl-btn");
let dlDiv = document.getElementById("download-file");
let dropZone = document.getElementById("dropzone");
let parentDiv = document.getElementById("parent");
let progress = document.getElementById("progress");
let status = document.getElementById("status");
let statusLog = document.getElementById("status-log");
let link = document.getElementById("link");
let filename = document.getElementById("filename");
let html = document.documentElement;
let fileName = "";
let fileSize = 1;
let receivedBytes = 0;

let fileUpload = document.getElementById("file-upload");
if (fileUpload.files[0] !== undefined) {
    nextStep(fileUpload.files[0]);
}
fileUpload.addEventListener("change", function(e) {
    let file = fileUpload.files[0];

    nextStep(file);
});

const STATE_WAITING = 0;
const STATE_SIGNALLING = 1;
const STATE_TRANSFERRING = 2;

let state = STATE_WAITING;

function isFile(evt) {
    var dt = evt.dataTransfer;

    for (var i = 0; i < dt.types.length; i++) {
        if (dt.types[i] === "Files") {
            return true;
        }
    }
    return false;
}

function setProgress(p) {
    if (p !== null) {
        // console.log(p);
        progress.setAttribute("value", p);
    } else {
        progress.removeAttribute("value");
    }
}

function setStatus(text) {
    status.innerText = text;
    status.classList.remove("error");
}

function setErrorStatus(text) {
    status.innerText = text;
    status.classList.add("error");
}

function dragEnter(ev) {
    console.log("enter!");
    console.log(ev.target);


    if (isFile(ev)) {
        parentDiv.style.borderColor = "black";
        dropZone.style.display = "block";

        ev.preventDefault();
    }
}

function dragEnd(ev) {
    console.log("end!");

    if (ev.target == html) {
        parentDiv.style.borderColor = "transparent";

    }
}

function dragLeave(ev) {
    console.log("leave!");
    console.log(ev.target);
    if (ev.target == dropzone) {
        parentDiv.style.borderColor = "transparent";

        dropZone.style.display = "none";
    }
}

function dragOver(ev) {
    console.log("over!");
    ev.preventDefault();
}

function dragDrop(ev) {
    ev.preventDefault();

    console.log("drop!");
    console.log(ev.dataTransfer.files);

    dropZone.style.display = "none";
    fileUpload.files = ev.dataTransfer.files;
    if (fileUpload.files.length > 0) {
        nextStep(fileUpload.files[0]);
    }
}

window.addEventListener("dragenter", dragEnter);
window.addEventListener("dragleave", dragLeave);
window.addEventListener("dragover", dragOver);
window.addEventListener("drop", dragDrop);

function nextStep(file) {
    fileToShare = file;
    parentDiv.style.borderColor = "transparent";
    chooseDiv.classList.add("disabled");
    shareDiv.classList.remove("disabled");
    shareButton.removeAttribute("disabled");
}

dlBtn.onclick = startDownload;
shareButton.onclick = share;

function startDownload() {
    dlDiv.style.display = "none";

    setProgress(null);
    setStatus("Establishing connection...");

    startFakeDownloadRequest(fileName, fileSize);
}

function download(href) {
    if (firefox) {
        var element = document.createElement('a');
        element.setAttribute('href', href);
        element.setAttribute('download', "");

        element.style.display = 'none';
        document.body.appendChild(element);

        element.click();

        document.body.removeChild(element);
    } else {
        location.href = href;
    }
}

function startFakeDownloadRequest(fileName, fileSize) {
    let name = encodeURIComponent(fileName.replace(/\//g, ':'))
        .replace(/['()]/g, escape)
        .replace(/\*/g, '%2A');

    let headers = {
        'Content-Type': 'application/octet-stream; charset=utf-8',
        'Content-Disposition': "attachment; filename*=UTF-8''" + name,
        'Content-Length': fileSize,
    };

    msgChannel.port1.onmessage = onServiceWorkerMessage;

    console.log("Posting StartDownload to service worker");
    navigator.serviceWorker.controller.postMessage(
        { type: 'StartDownload', 'fileName': fileName, 'headers': headers },
        [msgChannel.port2]);

    console.log("Posted StartDownload to service worker");

    // location.href = `${window.origin}/`;
}

function onServiceWorkerMessage(event) {
    console.log("Service worker message");
    console.log(event);

    let href = event.data;

    download(href);
    console.log("finished download dialog?");

    state = STATE_SIGNALLING;

    startWebRtc();
    setupReceiverChannel();

    send("StartSignalling");
}

function share() {
    chooseDiv.style.display = "none";
    shareDiv.style.display = "none";
    sharingDiv.style.display = "block";

    setProgress(null);
    setStatus("Creating link...");

    socket = new WebSocket(ws);
    socket.onclose = onUploaderSignallingSocketClosed;
    socket.onmessage = onMessage;

    socket.onopen = function() {
        send({ "UploadFile": {"name": fileToShare.name, "size": fileToShare.size } });
        uploading = true;

        statusDiv.style.display = "block";
        setStatus("Waiting for someone to click link...");
    };

}

function onUploaderSignallingSocketClosed(event) {
    if (state <= STATE_SIGNALLING) {
        setErrorStatus("Connection to signalling server lost");
        setProgress(0);
    }
}

function onDownloaderSignallingSocketClosed(event) {
    if (state == STATE_SIGNALLING) {
        setErrorStatus("Connection to signalling server lost");
        setProgress(0);
    }
}

function send(msg) {
    let json = JSON.stringify(msg);
    console.log(`=> ${json}`);

    socket.send(json);
}

async function onMessage(event) {
    let msg = JSON.parse(event.data);

    console.log("msg!");
    console.log(event.data);

    if (msg.UploadFileResponse != null) {
        onUploadFileResponse(msg.UploadFileResponse);
    } else if (msg.ParticipantJoin != null) {
        onParticipantJoin();
    } else if (msg.SessionInfo != null) {
        onSessionInfo(msg.SessionInfo);
    } else if (msg == "SessionFull") {
        onSessionFull();
    } else if (msg == "StartSignalling") {
        await onStartSignalling();
    } else if (msg.answer != null) {
        await onSdpAnswer(msg.answer);
    } else if (msg.offer != null) {
        await onSdpOffer(msg.offer);
    } else if (msg.candidate != null) {
        await onNewIceCandidate(msg.candidate);
    }
}

function onSessionInfo(info) {
    dlDiv.style.display = "block";

    fileName = info.file_name;
    fileSize = info.size;

    filename.innerText = `${info.file_name} - ${info.size} B`;

    setProgress(0);
    setStatus("");
}

function onSessionFull() {
}

async function onStartSignalling() {
    state = STATE_SIGNALLING;

    startWebRtc();
    await setupSenderChannel();
}

async function onSdpOffer(offer) {
    await peerConnection.setRemoteDescription(new RTCSessionDescription({ "type": "offer", "sdp": offer }));
    let answer = await peerConnection.createAnswer();
    await peerConnection.setLocalDescription(answer);

    send({ 'answer': answer.sdp });
}

async function onSdpAnswer(answer) {
    await peerConnection.setRemoteDescription(new RTCSessionDescription({ "type": "answer", "sdp": answer }));
}

async function onNewIceCandidate(candidate) {
    remoteIceCandidates++;
    setSignallingStatus();

    console.log(`Remote ICE candidate: ${candidate.candidate}`);

    await peerConnection.addIceCandidate(new RTCIceCandidate({
        'candidate': candidate.candidate,
        'sdpMid': candidate.sdpMid,
        'sdpMLineIndex': candidate.sdpMLineIndex,
        'usernameFragment': candidate.usernameFragment,
    }));
}

function onUploadFileResponse(response) {
    link.innerText = `${host}/#${response.link}`;
    link.href = `${href}#${response.link}`;
}

function startWebRtc() {
    const configuration = {'iceServers': [{'urls': 'stun:stun.l.google.com:19302'}]}
    peerConnection = new RTCPeerConnection(configuration);
    peerConnection.onicecandidate = onLocalIceCandidate;

    peerConnection.onconnectionstatechange = onConnectionStateChange;
    peerConnection.oniceconnectionstatechange = onIceConnectionStateChange;
    peerConnection.onicegatheringstatechange = onIceGatheringStateChange;
}

let peerConnectionState = "";
let iceConnectionState = "";
let iceGatheringState = "";
let localIceCandidates = 0;
let remoteIceCandidates = 0;

function onConnectionStateChange(event) {
    peerConnectionState = peerConnection.connectionState;
    setSignallingStatus();
}

function onIceConnectionStateChange(event) {
    iceConnectionState = peerConnection.iceConnectionState;
    setSignallingStatus();
}

function onIceGatheringStateChange(event) {
    iceGatheringState = peerConnection.iceGatheringState;
    setSignallingStatus();
}

function setSignallingStatus() {
    if (state == STATE_SIGNALLING) {
        status.innerHTML = `Peer: ${peerConnectionState} ICE: ${iceConnectionState}, ${iceGatheringState} (${localIceCandidates}/${remoteIceCandidates})`;
    }
}

function onUploaderDataChannelError(event) {
    console.log(event);

    setErrorStatus("Error while uploading");
}

function onDownloaderDataChannelError(event) {
    console.log(event);

    setErrorStatus("Error while downloading");
}

async function setupSenderChannel() {
    channel = peerConnection.createDataChannel("sendChannel");
    channel.onopen = channelStatusChange;
    channel.onclose = channelStatusChange;
    channel.onerror = onUploaderDataChannelError;

    let offer = await peerConnection.createOffer();
    await peerConnection.setLocalDescription(offer);
    send({ 'offer': offer.sdp });
}

function setupReceiverChannel() {
    peerConnection.ondatachannel = receiveChannel;
}

function receiveChannel(event) {
    state = STATE_TRANSFERRING;

    console.log("Receive channel:");
    console.log(event);

    channel = event.channel;
    channel.onerror = onDownloaderDataChannelError;
    channel.onmessage = onReceiveData;

    setStatus("Waiting for file to be sent");
}

async function onReceiveData(event) {
    // console.log(event);
    let blob = event.data;


    setStatus("Downloading...");


    let data = blob;
    if (blob.arrayBuffer !== undefined) {
        data = await blob.arrayBuffer();
    }

    receivedBytes += data.byteLength;

    // console.log(receivedBytes / fileSize);
    setProgress(receivedBytes / fileSize);

    // let data = await blob.arrayBuffer();
    data = new Uint8Array(data);
    let isDone = receivedBytes == fileSize;

    if (isDone) {
        msgChannel.port1.postMessage({ data: data, done: false });
        msgChannel.port1.postMessage({ data: null, done: true });
    } else {
        msgChannel.port1.postMessage({ data: data, done: false });
    }

    if (isDone) {
        // await writer.close();
        setStatus("Completed download");
    }

    // console.log(event.data);
}

let uploadInterval = null;

async function channelStatusChange(event) {
    console.log("Channel status change:");
    console.log(event);

    if (event.type == "open") {
        state = STATE_TRANSFERRING;
        setStatus("Uploading");

        console.log("start sending");

        const CHUNK_SIZE = 1 << 16;

        let i = 0;
        uploadInterval = setInterval(async function() {
            while (channel.bufferedAmount < 1024 * 1024) {
                let end = Math.min(i + CHUNK_SIZE, fileToShare.size);
                let blob = fileToShare.slice(i, end);

                console.log(end / fileToShare.size);
                await channel.send(blob);

                setProgress(end / fileToShare.size);

                i += CHUNK_SIZE;

                if (i >= fileToShare.size) {
                    clearInterval(uploadInterval);
                    setStatus("Finished uploading");
                    break;
                }
            }
        }, 25);

        /*for (let i = 0; i < fileToShare.size; i += CHUNK_SIZE) {
            let end = Math.min(i + CHUNK_SIZE, fileToShare.size);
            let blob = fileToShare.slice(i, end);

            await channel.send(blob);

            setProgress(end / fileToShare.size);
        }*/
    }
}

function onLocalIceCandidate(event) {
    let c = event.candidate;
    if (c) {
        localIceCandidates++;
        setSignallingStatus();

        console.log(`Local ICE candidate: ${c.candidate}`);

        send({
            'candidate': {
                'candidate': c.candidate,
                'sdpMid': c.sdpMid,
                'sdpMLineIndex': c.sdpMLineIndex,
                'usernameFragment': c.usernameFragment,
            }
        });
    }
}

function downloadFromFragment(fragment) {
    setStatus("Connecting to session...");

    socket = new WebSocket(ws);
    socket.onmessage = onMessage;
    socket.onclose = onDownloaderSignallingSocketClosed;

    socket.onopen = function() {
        send({ "JoinSession": {"id": fragment} });

        downloading = true;
    };
}

let fragment = window.location.hash.substr(1);
if (fragment.length > 0) {
    chooseDiv.style.display = "none";
    shareDiv.style.display = "none";
    sharingDiv.style.display = "block";

    downloadFromFragment(fragment);
}
else {
    console.log("No fragment!");
}

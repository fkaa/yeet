let fileUpload = document.getElementById("file-upload");
fileUpload.addEventListener("change", function(e) {
    let file = fileUpload.files[0];

    console.log(file);
});

SignallingChannel = function(address) {
    this.ws = new WebSocket(address);
}

let fragment = window.location.hash.substr(1);
if (fragment.length > 0) {
    signallingChannel = new SignallingChannel(`ws://localhost:8080/signalling/${fragment}`);

}
else {
    console.log("No fragment!");
}

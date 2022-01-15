let fileUpload = document.getElementById("file-upload");
fileUpload.addEventListener("change", function(e) {
    let file = fileUpload.files[0];

    console.log(file);
});

SignallingChannel = function(address) {
    this.ws = new WebSocket(address);
}

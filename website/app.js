let fileUpload = document.getElementById("file-upload");
fileUpload.addEventListener("change", function(e) {
    let file = fileUpload.files[0];

    console.log(file);
});

let chooseDiv = document.getElementById("choose");
let shareDiv = document.getElementById("share");
let shareButton = document.getElementById("share-btn");
let dropZone = document.getElementById("dropzone");
let html = document.documentElement;

function isFile(evt) {
    var dt = evt.dataTransfer;

    for (var i = 0; i < dt.types.length; i++) {
        if (dt.types[i] === "Files") {
            return true;
        }
    }
    return false;
}

function dragEnter(ev) {
    console.log("enter!");
    console.log(ev.target);


    if (isFile(ev)) {
        html.classList.add("border");
        dropZone.style.display = "block";

        ev.preventDefault();
    }
}

function dragEnd(ev) {
    console.log("end!");

    if (ev.target == html) {
        html.classList.remove("border");

    }
}

function dragLeave(ev) {
    console.log("leave!");
    console.log(ev.target);
    if (ev.target == dropzone) {
        html.classList.remove("border");

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
        nextStep();
    }
}

window.addEventListener("dragenter", dragEnter);
window.addEventListener("dragleave", dragLeave);
window.addEventListener("dragover", dragOver);
window.addEventListener("drop", dragDrop);

//html.ondragenter = dragEnter;
//html.ondragend = dragEnd;
//html.ondragleave = dragLeave;


function nextStep() {
    html.classList.remove("border");
    chooseDiv.classList.add("disabled");
    shareDiv.classList.remove("disabled");
    shareButton.removeAttribute("disabled");
}

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

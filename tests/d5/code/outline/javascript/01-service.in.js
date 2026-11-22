class Service {
  start() {
    const log = (message) => console.log(message);
    log("start");
  }

  stop() {}
}

function main() {
  new Service().start();
}

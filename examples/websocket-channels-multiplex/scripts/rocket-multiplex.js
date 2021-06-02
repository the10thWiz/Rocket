
class RocketMultiplex {
  static _connection = {
    connections: [],
    topics: [],
  };
  constructor(hostname, topic) {
    //this.binaryType;
    //this.bufferedAmount;
    //this.extensions;
    //this.onclose = () => {};
    //this.onerror = () => {};
    //this.onmessage = () => {};
    //this.onopen = () => {};
    //this.protocol;
    //this.readyState;
    this.url = "wss://" + hostname + topic;
    for(let i = 0; i < RocketMultiplex._connection.connections.length; i++) {
      if(RocketMultiplex._connection.connections[i].hostname == hostname && RocketMultiplex._connection.connections[i].topic == topic) {
        // add object
        RocketMultiplex._connection.topics.push(this);
        return;
      }
    }
    // else
    RocketMultiplex._connection.connections.push({
      hostname: hostname,
      topic: topic,
      connection: new WebSocket(this.url, 'rocket-multiplex'),
    });
  }
  /// Read only
  get url() {
    return this.url;
  }
  set url() {
    console.error("Url is readonly");
  }
  addEventListener(event, callback) {
  }
  /// code: int?, reason: string?
  close(code, reason) {
  }
  send(message) {
  }
}

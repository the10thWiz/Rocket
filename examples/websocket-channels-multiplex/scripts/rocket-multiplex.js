const RocketWebsocket = (() => {
  const seperator = '\u{B7}';
  class ConnectionPool {
    constructor() {
      this.ws = [];
    }
    // open, error, and close all just forward the event to every listening websocket object
    forward_event(e) {
      console.log(e);
      for (let i = 0; i < this.ws.length; i++) {
        if (this.ws[i].w == e.target) {
          for (let j = 0; j < this.ws[i].listeners.length; j++) {
            this.ws[i].listeners[j].dispatchEvent(e);
          }
          return;
        }
      }
    }
    forward_topic_event(e, topic) {
      console.log(e);
      for (let i = 0; i < this.ws.length; i++) {
        if (this.ws[i].w == e.target) {
          for (let j = 0; j < this.ws[i].listeners.length; j++) {
            let url = new URL(this.ws[i].listeners[j].url);
            if (topic == url.pathname + url.search) {
              this.ws[i].listeners[j].dispatchEvent(e);
            }
          }
          return;
        }
      }
    }
    // This needs to actually parse stuff
    onmessage(e) {
      console.log(e);
      for (let i = 0; i < this.ws.length; i++) {
        if (this.ws[i].w == e.target) {
          let host = e.origin;
          let topic;
          let data;
          if (typeof e.data == 'string') {
            let idx = e.data.indexOf(seperator);
            if (idx == -1 || idx > 100) {
              console.error('Could not parse message');
            } else if (idx == 0) {
              // Control message
              let parts = e.data.split(seperator).filter((s) => s != '');
              switch (parts[0]) {
                case 'INVALID':
                  console.error(`Invalid: ${parts[1]}`);
                  break;
                case 'ERR':
                  switch (parts[2]) {
                    case 'SUBSCRIBE':
                      let idx = this.ws[i].topics.indexOf(parts[3]);
                      if (idx != -1) {
                        this.ws[i].topics.splice(idx);
                      }
                      this.forward_topic_event(new Event('error'), parts[3]);
                      this.forward_topic_event(new Event('close'), parts[3]);
                      break;
                    case 'UNSUBSCRIBE':
                      this.forward_topic_event(new Event('error'), parts[3]);
                      let idx = this.ws[i].topics.indexOf(parts[3]);
                      if (idx != -1) {
                        this.ws[i].topics.splice(idx);
                      }
                      break;
                    default:
                      console.error('Invalid action');
                  }
                  break;
                case 'OK':
                  switch (parts[1]) {
                    case 'SUBSCRIBE':
                      this.ws[i].topics.push(parts[2]);
                      this.forward_topic_event(new Event('open'), parts[2]);
                      break;
                    case 'UNSUBSCRIBE':
                      this.forward_topic_event(new Event('close'), parts[2]);
                      let idx = this.ws[i].topics.indexOf(parts[2]);
                      if (idx != -1) {
                        this.ws[i].topics.splice(idx);
                      }
                      break;
                    default:
                      console.error('Invalid action');
                  }
                  break;
                default:
                  console.error('Invalid control frame');
              }
              return;
            } else {
              topic = e.data.slice(0, idx);
              data = e.data.slice(idx + seperator.length);
            }
          } else if (typeof e.data == 'object') {
            console.error('binary messages are not implemented');
          } else {
            return;
          }
          let ev = new MessageEvent('message', {
            data: data,
            origin: host,
            lastEventId: e.lastEventId,
          });
          let url = host + topic;
          for (let j = 0; j < this.ws[i].listeners.length; j++) {
            if (this.ws[i].listeners[j].url == url) {
              this.ws[i].listeners[j].dispatchEvent(ev);
            }
          }
          return;
        }
      }
    }
    // Method to add listening websocket
    add(w) {
      console.log(this);
      let url = new URL(w.url);
      // TODO handle events, etc
      for(let i = 0; i < this.ws.lengthis; i++) {
        if (url.host == this.ws[i].url.host && url.protocol == this.ws[i].url.protocol) {
          this.ws[i].listeners.push(w)
          this.ws[i].send(seperator + 'SUBSCRIBE' + seperator + url.pathname + url.search);
          return;
        }
      }
      let ws = new WebSocket(w.url, 'rocket-multiplex');
      ws.onopen = this.forward_event.bind(this);
      ws.onclose = this.forward_event.bind(this);
      ws.onerror = this.forward_event.bind(this);
      ws.onmessage = this.onmessage.bind(this);
      this.ws.push({
        url: url,
        w: ws,
        topics: [url.pathname + url.search],
        listeners: [w],
      });
    }
    send(m, u) {
      let url = new URL(u);
      let topic = url.pathname + url.search;
      console.log(topic);
      if (typeof m == 'string') {
        for(let i = 0; i < this.ws.length; i++) {
          if (url.origin == this.ws[i].url.origin) {
            if (this.ws[i].topics.indexOf(topic) != -1) {
              console.log('sending');
              this.ws[i].w.send(topic + seperator + m);
              return;
            }
          }
        }
      } else if (typeof m == 'object') {
        console.error('Bindary types are not implemented');
      }
    }
    close(code, reason, u) {
      let url = new URL(u);
      let topic = url.pathname + url.search;
      for(let i = 0; i < this.ws.length; i++) {
        if (url.host == this.ws[i].url.host && url.protocol == this.ws[i].url.protocol) {
          let idx = this.ws[i].topics.findIndex((t) => t == topic);
          if (idx != -1) {
            this.ws[i].send(seperator + 'UNSUBSCRIBE' + seperator + url.pathname + url.search + seperator + code + ' ' + reason);
            return;
          }
        }
      }
    }
  };
  let connection_pool = new ConnectionPool();

  return class extends EventTarget {
    constructor(url) {
      super();
      //console.log('created');
      this.url = url;
      this.listeners = {};
      connection_pool.add(this);
    }

    // Properties
    //get binaryType() {
      //return this._inner.binaryType;
    //}
    // This is an issue I will look into later
    //set binaryType(binaryType) {
      //this._inner.binaryType = binaryType;
    //}
    // Is this useful?
    //get bufferedAmount() {
      //return this._inner.bufferedAmount;
    //}
    //get extensions() {
      //return this._inner.extensions;
    //}
    // Rocket multiplex doesn't implement protocols yet
    //get protocol() {
      //return this._inner.protocol;
    //}
    // Will be ignored for now
    //get readyState() {
      //return this._inner.readyState;
    //}
    //get url() {
      //return this.url;
    //}
    // Disable modication to url
    //set url(_u) {
    //}

    // Methods
    close(code, reason) {
      //return this._inner.close(code, reason);
      connection_pool.close(code, reason, this.url);
    }

    send(message) {
      connection_pool.send(message, this.url);
    }

    //EventTarget.prototype.listeners = null;
    //get listeners() {
      //return {};
    //}
    //set listeners(_l) {
      //return {};
    //}

    // Events TODO: options, etc
    addEventListener(type, callback) {
      if (!(type in this.listeners)) {
        this.listeners[type] = [];
      }
      this.listeners[type].push(callback);
    }

    removeEventListener(type, callback) {
      if (!(type in this.listeners)) {
        return;
      }
      var stack = this.listeners[type];
      for (var i = 0, l = stack.length; i < l; i++) {
        if (stack[i] === callback){
          stack.splice(i, 1);
          return;
        }
      }
    }

    dispatchEvent(event) {
      if (!(event.type in this.listeners)) {
        return true;
      }
      var stack = this.listeners[event.type].slice();

      for (var i = 0, l = stack.length; i < l; i++) {
        stack[i].call(this, event);
      }
      return !event.defaultPrevented;
    }

    get onclose() {
      return this.onclose;
    }
    set onclose(onclose) {
      if (this.onclose) {
        this.removeEventListener('close', this.onclose);
      }
      this.addEventListener('close', onclose);
      return this.onclose = onclose;
    }

    get onerror() {
      return this.onerror;
    }
    set onerror(onerror) {
      if (this.onerror) {
        this.removeEventListener('error', this.onerror);
      }
      this.addEventListener('error', onerror);
      return this.onerror = onerror;
    }

    get onmessage() {
      return this.onmessage;
    }
    set onmessage(onmessage) {
      if (this.onmessage) {
        this.removeEventListener('message', this.onmessage);
      }
      this.addEventListener('message', onmessage);
      return this.onmessage = onmessage;
    }

    get onopen() {
      return this.onopen;
    }
    set onopen(onopen) {
      if (this.onopen) {
        this.removeEventListener('open', this.onopen);
      }
      this.addEventListener('open', onopen);
      return this.onopen = onopen;
    }
  };
})();

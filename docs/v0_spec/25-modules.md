## 25. Modules

A `.zt` file can return a record containing values, functions, and types.

Example `server.zt`:

```zt
let RawServer: Type = type {
  host? = Text;
  port? = Int;
  tls? = Bool;
}

let Server: Type = type {
  host = Text;
  port = Int;
  tls = Bool;
}

let normalize: RawServer -> Server =
  fn raw => {
    host = raw.host ?? "127.0.0.1";
    port = raw.port ?? 8080;
    tls = raw.tls ?? false;
  }

{
  RawServer = RawServer;
  Server = Server;
  normalize = normalize;
}
```

Another file:

```zt
let serverLib = import "server.zt"
let raw = import "server.zti"

let server: serverLib.Server =
  serverLib.normalize raw

server
```

This works because imported `.zt` modules can contain non-serializable values like functions and types.

Only rendering requires serializability.

---


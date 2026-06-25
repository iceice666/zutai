## Modules

A `.zt` file can return a record containing values, functions, and types.

Example `server.zt`:

```zt
RawServer :: type {
  host? : Text;
  port? : Int;
  tls? : Bool;
};

Server :: type {
  host : Text;
  port : Int;
  tls : Bool;
};

normalize :: RawServer -> Server
  = raw => {
    host = raw.host ?? "127.0.0.1";
    port = raw.port ?? 8080;
    tls = raw.tls ?? false;
  };

{
  RawServer = RawServer;
  Server = Server;
  normalize = normalize;
}
```

Another file:

```zt
serverLib :: import "server.zt";
raw :: import "server.zti";

server :: serverLib.Server = serverLib.normalize raw;

server
```

This works because imported `.zt` modules can contain non-serializable values like functions and types, and import declarations expose them through the chosen prefix.

Only rendering requires serializability.

---

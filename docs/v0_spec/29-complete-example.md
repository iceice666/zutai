## 29. Complete example

### 29.1 `app.zti`

```zti
{
  name = "demo";
  profile = #prod;

  server = {
    host = "localhost";
    port = 8080;
  };
}
```

### 29.2 `app.zt`

```zt
let Profile: Type = type [
  #dev;
  #test;
  #prod;
]

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

let RawConfig: Type = type {
  name = Text;
  profile = Profile;
  server = RawServer;
}

let Config: Type = type {
  name = Text;
  profile = Profile;
  server = Server;
}

let raw: RawConfig = import "app.zti"

let normalizeServer: RawServer -> Server =
  fn server => {
    host = server.host ?? "127.0.0.1";
    port = server.port ?? 8080;
    tls = server.tls ?? false;
  }

let config: Config = {
  name = raw.name;
  profile = raw.profile;
  server = normalizeServer raw.server;
}

config
```

The final expression:

```zt
config
```

is the file output.

### 29.3 Optional chaining example

`nested.zti`:

```zti
{
  server = {
    host = "localhost";
  };
}
```

`nested.zt`:

```zt
let RawConfig: Type = type {
  server? = {
    host? = Text;
    port? = Int;
  };
}

let raw: RawConfig = import "nested.zti"

{
  host = raw.server?.host ?? "127.0.0.1";
  port = raw.server?.port ?? 8080;
}
```

---


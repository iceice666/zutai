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
Profile :: type [
  #dev;
  #test;
  #prod;
]

RawServer :: type {
  host? : Text;
  port? : Int;
  tls?  : Bool;
}

Server :: type {
  host : Text;
  port : Int;
  tls  : Bool;
}

RawConfig :: type {
  name    : Text;
  profile : Profile;
  server  : RawServer;
}

Config :: type {
  name    : Text;
  profile : Profile;
  server  : Server;
}

raw : RawConfig = import "app.zti"

normalizeServer :: RawServer -> Server
               :: s {
                 {
                   host = s.host ?? "127.0.0.1";
                   port = s.port ?? 8080;
                   tls  = s.tls  ?? false;
                 }
               }

config : Config = {
  name    = raw.name;
  profile = raw.profile;
  server  = normalizeServer raw.server;
}

config
```

The final expression `config` is the file output.

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
RawConfig :: type {
  server? : {
    host? : Text;
    port? : Int;
  };
}

raw : RawConfig = import "nested.zti"

{
  host = raw.server?.host ?? "127.0.0.1";
  port = raw.server?.port ?? 8080;
}
```

### 29.4 Tuple variant and pattern matching example

```zt
Shape :: type [
  (#circle, radius : Float);
  (#square, length : Float);
  (#rect,   width  : Float, height : Float);
]

area :: Shape -> Float
     :: (#circle, radius = r)          { r * r * 3.14159 }
     :: (#square, length = l)          { l * l }
     :: (#rect, width = w, height = h) { w * h }

shapes : List Shape = [
  (#circle, radius = 1.0);
  (#square, length = 2.0);
  (#rect, width = 3.0, height = 4.0);
]

total_area := fold (\acc s => acc + area s) 0.0 shapes
```

---

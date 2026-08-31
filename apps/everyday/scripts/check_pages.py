import json
import urllib.request

from runtime_config import require_http_base

base = require_http_base()
print("health", urllib.request.urlopen(base + "/health", timeout=10).read().decode())
for path in ("/", "/login", "/join"):
    req = urllib.request.Request(base + path, headers={"User-Agent": "Mozilla/5.0"})
    r = urllib.request.urlopen(req, timeout=20)
    body = r.read()
    print(path, r.status, r.getheader("Content-Type"), len(body), b"index-" in body)
req = urllib.request.Request(
    base + "/api/trpc/auth.directory?batch=1",
    data=json.dumps({"0": {"json": {}}}).encode(),
    headers={"Content-Type": "application/json"},
    method="POST",
)
raw = json.loads(urllib.request.urlopen(req, timeout=20).read().decode())
d = raw[0]["result"]["data"]["json"]
print("users", len(d), [u["fullName"] for u in d])
print("no_demo", all(u.get("phone") != "+7 921 555-01-42" for u in d))

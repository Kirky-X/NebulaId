# TLS 证书目录

TLS/HTTPS 部署时把证书放入本目录（compose 已挂载到容器 `/app/certs:ro`）：

- `server.crt` — 服务器证书链（PEM）
- `server.key` — 服务器私钥（PEM，PKCS#8/RSA/EC）
- `ca.crt` — 可选，自定义 CA（mTLS/客户端校验）

未启用 TLS（默认）时本目录为空，挂载无副作用。生成自签证书（仅测试/内网）：

```bash
openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
  -keyout docker/certs/server.key -out docker/certs/server.crt \
  -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

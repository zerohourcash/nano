from runtime_config import connect_ssh

c = connect_ssh()
cmd = (
    "systemctl is-active meshkeeper nginx; "
    "curl -sS --max-time 3 http://127.0.0.1:8080/health; echo; "
    "curl -sS --max-time 3 -o /dev/null -w '%{http_code} %{size_download}\\n' http://127.0.0.1/; "
    "ss -tlnp | grep -E ':80|:8080'"
)
stdin, stdout, stderr = c.exec_command(cmd, timeout=20)
print(stdout.read().decode())
print(stderr.read().decode()[-800:])
c.close()

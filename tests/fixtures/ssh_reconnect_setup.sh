# Called by the opt-in WSL terminal test in a fresh mode-0700 directory.
set -eu
cd "$1"
base=$PWD
ssh-keygen -q -t ed25519 -N '' -f host
ssh-keygen -q -t ed25519 -N '' -f client
cat >server.conf <<EOF
HostKey $base/host
AuthorizedKeysFile $base/client.pub
PidFile $base/pid
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
StrictModes no
AllowAgentForwarding no
AllowTcpForwarding no
X11Forwarding no
ForceCommand $base/login
EOF
cat >login <<EOF
#!/bin/sh
unset TMUX
export HOME=$base XDG_CONFIG_HOME=$base TMUX_TMPDIR=$base SHELL=/bin/sh
exec sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
cat >proxy <<'PYTHON'
#!/usr/bin/python3
# This relay owns the client's pipes. Killing it drops the connection even
# when the authenticated sshd process has forked and still owns its socket.
import os
import socket
import subprocess
import sys
import threading

root = os.path.dirname(__file__)
client, server = socket.socketpair()
daemon = subprocess.Popen(
    ["/usr/sbin/sshd", "-i", "-f", root + "/server.conf", "-E", root + "/server.log"],
    stdin=server, stdout=server,
)
server.close()
with open(root + "/transport", "w", encoding="ascii") as pid_file:
    pid_file.write(str(os.getpid()))

def copy_input():
    while data := os.read(0, 65536):
        client.sendall(data)
    client.shutdown(socket.SHUT_WR)

threading.Thread(target=copy_input, daemon=True).start()
while data := client.recv(65536):
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()
client.close()
daemon.wait()
PYTHON
chmod 700 login proxy
cat >config <<EOF
Host fixture
 HostName 127.0.0.1
 User $(id -un)
 IdentityFile $base/client
 IdentitiesOnly yes
 BatchMode yes
 StrictHostKeyChecking yes
 UserKnownHostsFile $base/known
 GlobalKnownHostsFile /dev/null
 ProxyCommand $base/proxy
EOF
awk '{print "127.0.0.1", $1, $2}' host.pub >known

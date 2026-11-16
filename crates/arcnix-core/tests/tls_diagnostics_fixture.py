"""Finite private TLS peers; no host CA store, accounts or services are changed."""
import json, pathlib, socket, ssl, subprocess, sys, threading, time

root=pathlib.Path(sys.argv[1]);assert str(root).startswith('/tmp/arcnix-tls-diagnostics-') and root.resolve()==root

def openssl(*args):
    with (root/'openssl.log').open('ab') as log:
        subprocess.run(['openssl',*map(str,args)],check=True,stdout=log,stderr=log)

openssl('req','-x509','-newkey','rsa:2048','-nodes','-subj','/CN=Arcnix private TLS fixture','-days','90',
    '-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign','-keyout',root/'ca.key','-out',root/'ca.pem')
openssl('genpkey','-algorithm','RSA','-pkeyopt','rsa_keygen_bits:2048','-out',root/'leaf.key')
for name,sans,days in [('default','DNS:default.test,IP:127.0.0.1,IP:::1',90),('sni','DNS:sni.test',90),('soon','DNS:soon.test',1),('expired','DNS:expired.test',-1)]:
    (root/'extensions').write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName='+sans+'\n')
    openssl('req','-new','-key',root/'leaf.key','-subj','/CN='+name+'.test','-out',root/'leaf.req')
    openssl('x509','-req','-in',root/'leaf.req','-CA',root/'ca.pem','-CAkey',root/'ca.key','-CAcreateserial','-days',days,'-extfile',root/'extensions','-out',root/(name+'.pem'))
openssl('req','-x509','-key',root/'leaf.key','-subj','/CN=untrusted.test','-days','90','-addext','subjectAltName=DNS:untrusted.test','-out',root/'untrusted.pem')

def context(name):
    value=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);value.load_cert_chain(root/(name+'.pem'),root/'leaf.key');return value

contexts={name:context(name) for name in ['default','sni','soon','expired','untrusted']}
seen=[];lock=threading.Lock()
def sni(conn,name,initial):
    with lock:seen.append(name);(root/'sni.json').write_text(json.dumps(seen))
    if name=='sni.test':conn.context=contexts['sni']
contexts['default'].set_servername_callback(sni)
sockets=[];ports={}
def serve(name,ctx=None,family=socket.AF_INET):
    sock=socket.socket(family);sock.bind(('::1' if family==socket.AF_INET6 else '127.0.0.1',0));sock.listen(20);sockets.append(sock);ports[name]=sock.getsockname()[1]
    def handle(conn):
        with conn:
            conn.settimeout(25)
            try:
                if name=='hold':time.sleep(25)
                elif name=='plain':conn.sendall(b'HTTP/1.0 400 Bad Request\r\n\r\n')
                else:
                    with ctx.wrap_socket(conn,server_side=True) as tls:
                        tls.recv(1)
                        # Complete TLS shutdown instead of racing a bare socket
                        # close against the client's verification result.
                        tls.unwrap().close()
            except (OSError,ssl.SSLError):pass
    def loop():
        while True:
            try:conn,_=sock.accept()
            except OSError:return
            threading.Thread(target=handle,args=(conn,),daemon=True).start()
    threading.Thread(target=loop,daemon=True).start()

serve('good',contexts['default'])
for name in ['soon','expired','untrusted']:serve(name,contexts[name])
serve('plain');serve('hold')
serve('ipv6',contexts['default'],socket.AF_INET6)
closed=socket.socket();closed.bind(('127.0.0.1',0));sockets.append(closed);ports['closed']=closed.getsockname()[1]
(root/'ready.json').write_text(json.dumps(dict(ports=ports,openssl=ssl.OPENSSL_VERSION)))
time.sleep(150)
for sock in sockets:sock.close()

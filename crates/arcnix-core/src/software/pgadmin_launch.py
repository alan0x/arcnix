# Start pgAdmin's desktop-mode backend for one Arcnix window. Runs inside the
# managed component with its own data folder. The launch request arrives on
# stdin so the window key never appears in arguments or the parent environment.
import hashlib
import json
import os
import runpy
import sqlite3
import subprocess
import sys
import threading

GROUP = 'Arcnix'


def configuration(data, save_passwords):
    paths = {
        'DATA_DIR': data,
        'LOG_FILE': os.path.join(data, 'pgadmin4.log'),
        'SQLITE_PATH': os.path.join(data, 'pgadmin4.db'),
        'SESSION_DB_PATH': os.path.join(data, 'sessions'),
        'STORAGE_DIR': os.path.join(data, 'storage'),
        'AZURE_CREDENTIAL_CACHE_DIR': os.path.join(data, 'azure'),
        'KERBEROS_CCACHE_DIR': os.path.join(data, 'kerberos'),
    }
    settings = {
        **paths,
        'SERVER_MODE': False,
        'MASTER_PASSWORD_REQUIRED': False,
        # Arcnix keeps the key for saved passwords in the system keychain
        # and hands it over in the launch request (see password_key).
        # pgAdmin's own keychain storage stays off: its desktop check can
        # switch it off mid-session, which changes the key in use.
        'USE_OS_SECRET_STORAGE': False,
        'ALLOW_SAVE_PASSWORD': save_passwords,
        # Arcnix's own SSH connection replaces pgAdmin's tunnels.
        'ALLOW_SAVE_TUNNEL_PASSWORD': False,
        # Arcnix chooses the tested version.
        'UPGRADE_CHECK_ENABLED': False,
    }
    path = os.path.join(data, 'config_distro.py')
    with open(path, 'w', encoding='utf-8') as file:
        for name, value in settings.items():
            file.write(f'{name} = {value!r}\n')
    os.environ['CONFIG_DISTRO_FILE_PATH'] = path
    return paths['SQLITE_PATH']


def update_server(database, server):
    """Point the saved entry at this session's tunnel. True when it exists."""
    if not os.path.exists(database):
        return False
    with sqlite3.connect(database, timeout=10) as connection:
        tables = {row[0] for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table'")}
        if not {'server', 'servergroup'} <= tables:
            return False
        changed = connection.execute(
            'UPDATE server SET host = ?, port = ?, maintenance_db = ?, username = ? '
            'WHERE id = (SELECT server.id FROM server JOIN servergroup '
            'ON servergroup.id = server.servergroup_id '
            'WHERE servergroup.name = ? AND server.name = ? ORDER BY server.id LIMIT 1)',
            ('127.0.0.1', server['port'], server['database'], server['user'],
             GROUP, server['name']))
        return changed.rowcount == 1


def forget_other_passwords(database, data, key):
    """Saved passwords only decrypt with the key that saved them. When the
    key changed (a new keychain item, another computer, or no keychain),
    forget them so pgAdmin asks again instead of failing to decrypt."""
    path = os.path.join(data, 'password-key')
    check = hashlib.sha256(b'arcnix pgadmin password key\0' + key.encode()).hexdigest() \
        if key else 'none'
    try:
        with open(path, encoding='ascii') as file:
            if file.read() == check:
                return
    except FileNotFoundError:
        pass
    with sqlite3.connect(database, timeout=10) as connection:
        connection.execute('UPDATE server SET password = NULL, tunnel_password = NULL, '
                           'save_password = 0')
    with open(path + '.tmp', 'w', encoding='ascii') as file:
        file.write(check)
    os.replace(path + '.tmp', path)


def password_key(app, key):
    """Encrypt saved passwords with Arcnix's key. pgAdmin prefers a key in
    its key manager over every fallback, so set it before each request
    that has the desktop user signed in."""
    from flask_login import current_user

    @app.before_request
    def arcnix_password_key():
        if current_user.is_authenticated and app.keyManager.get() != key:
            app.keyManager.set(key)


def add_server(root, data, server):
    path = os.path.join(data, 'servers.json')
    with open(path, 'w', encoding='utf-8') as file:
        json.dump({'Servers': {'1': {
            'Name': server['name'],
            'Group': GROUP,
            'Host': '127.0.0.1',
            'Port': server['port'],
            'MaintenanceDB': server['database'],
            'Username': server['user'],
            'ConnectionParameters': {'sslmode': 'prefer', 'connect_timeout': 15},
        }}}, file)
    # The backend has no console on Windows; without this flag each setup
    # step would open a console window.
    hidden = getattr(subprocess, 'CREATE_NO_WINDOW', 0)
    for command in (['setup-db'], ['load-servers', path]):
        subprocess.run([sys.executable, '-s', '-B', os.path.join(root, 'setup.py'), *command],
                       stdin=subprocess.DEVNULL, check=True, cwd=data, creationflags=hidden)


def watch_parent():
    # Arcnix holds the other end open; it closes even if Arcnix crashes.
    while os.read(0, 4096):
        pass
    os._exit(0)


def main():
    os.umask(0o077)
    request = json.loads(sys.stdin.buffer.readline())
    threading.Thread(target=watch_parent, daemon=True).start()
    root, data, server = request['root'], request['data'], request['server']
    key = request.get('password_key')
    database = configuration(data, bool(key))
    if not update_server(database, server):
        add_server(root, data, server)
        if not update_server(database, server):
            raise SystemExit('pgAdmin did not save the server connection')
    forget_other_passwords(database, data, key)
    os.environ['PGADMIN_INT_KEY'] = request['key']
    os.environ['PGADMIN_SERVER_MODE'] = 'OFF'
    sys.path.insert(0, root)
    program = os.path.join(root, 'pgAdmin4.py')
    sys.argv = [program]
    from werkzeug.serving import WSGIRequestHandler, make_server

    class RequestHandler(WSGIRequestHandler):
        def log_request(self, code='-', size='-'):
            # The initial WebView URL contains the private desktop key.
            pass

    # Keep the same socket from port selection through the last request.
    # The Socket.IO middleware is part of pgAdmin's initialized WSGI app.
    with make_server('127.0.0.1', 0, None, threaded=True,
                     request_handler=RequestHandler) as backend:
        port = backend.server_port
        os.environ['PGADMIN_INT_PORT'] = str(port)
        app = runpy.run_path(program)['app']
        if key:
            password_key(app, key)
        backend.app = app
        path = os.path.join(data, 'backend.ready')
        with open(path + '.tmp', 'w', encoding='ascii') as file:
            file.write(f"{request['launch']} {port}")
        os.replace(path + '.tmp', path)
        backend.serve_forever()


main()

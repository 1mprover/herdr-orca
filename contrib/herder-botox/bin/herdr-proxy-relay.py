#!/usr/bin/env python3
"""Local proxy relay in front of the upstream proxy that ~/.zshrc exports.

A running process cannot be handed a new HTTPS_PROXY, so sessions are pointed
once at this relay instead (proxy-install.sh puts it into the env block of
~/.claude/settings.json) and only the relay ever learns the real upstream. It
reads it from ~/.zshrc through herdr-proxy-upstream.sh at start and on every
SIGHUP, which is what prefix+u sends. A reload also drops every open tunnel, so
sessions stuck on a dead upstream reconnect through the new one right away
instead of waiting out their own timeouts.

Status for the tab row goes to $XDG_RUNTIME_DIR/herdr-proxy/status, one line:

    <pid> <generation> <reloaded_at> <state> <since> <upstream> <error>

Upstream credentials stay in memory: neither the status line nor the log
carries them.
"""
import asyncio
import base64
import errno
import os
import signal
import socket
import sys
import time
import urllib.parse

BIN = os.path.dirname(os.path.abspath(__file__))
UPSTREAM_CMD = os.path.join(BIN, 'herdr-proxy-upstream.sh')
LISTEN_HOST = '127.0.0.1'
LISTEN_PORT = int(os.environ.get('HERDR_PROXY_PORT') or 18118)
STATE_DIR = os.path.join(os.environ.get('XDG_RUNTIME_DIR') or '/tmp', 'herdr-proxy')
STATUS_FILE = os.path.join(STATE_DIR, 'status')
PROBE = os.environ.get('HERDR_PROXY_PROBE') or 'api.anthropic.com:443'
RECHECK_SECONDS = 30
CONNECT_TIMEOUT = 15
HEAD_TIMEOUT = 30
HEAD_LIMIT = 64 * 1024
CHUNK = 64 * 1024
HOP_BY_HOP = {'proxy-authorization', 'proxy-connection', 'connection', 'keep-alive'}
REASONS = {400: 'Bad Request', 502: 'Bad Gateway'}


def log(message):
    print(message, file=sys.stderr, flush=True)


def split_host_port(target, default_port):
    host, sep, port = target.rpartition(':')
    if not sep or not port.isdigit():
        return target.strip('[]'), default_port
    return host.strip('[]'), int(port)


def describe(error):
    # TimeoutError and gaierror are OSError subclasses, so the order matters.
    if isinstance(error, asyncio.TimeoutError):
        return 'таймаут'
    if isinstance(error, socket.gaierror):
        return 'DNS'
    if isinstance(error, ConnectionRefusedError):
        return 'отказ'
    if isinstance(error, (ConnectionResetError, asyncio.IncompleteReadError)):
        return 'обрыв'
    if isinstance(error, OSError) and error.errno:
        return errno.errorcode.get(error.errno, str(error.errno))
    return type(error).__name__


def status_code(head):
    parts = head.split(b' ', 2)
    return int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0


class Upstream:
    def __init__(self, url):
        self.host = None
        self.port = None
        self.auth = None
        if not url:
            return
        parts = urllib.parse.urlsplit(url if '://' in url else 'http://' + url)
        if parts.scheme.lower() != 'http' or not parts.hostname:
            raise ValueError(f'only an http:// upstream can be relayed, got {parts.scheme}://')
        self.host = parts.hostname
        self.port = parts.port or 80
        if parts.username is not None:
            credentials = urllib.parse.unquote(parts.username) + ':' + urllib.parse.unquote(parts.password or '')
            self.auth = 'Basic ' + base64.b64encode(credentials.encode()).decode()

    @property
    def direct(self):
        return self.host is None

    @property
    def label(self):
        return 'direct' if self.direct else f'{self.host}:{self.port}'

    async def open(self):
        return await asyncio.wait_for(asyncio.open_connection(self.host, self.port), CONNECT_TIMEOUT)

    async def connect(self, reader, writer, target):
        request = f'CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n'
        if self.auth:
            request += f'Proxy-Authorization: {self.auth}\r\n'
        writer.write((request + '\r\n').encode('latin-1'))
        await writer.drain()
        head = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), HEAD_TIMEOUT)
        return status_code(head), head


class Relay:
    def __init__(self):
        self.upstream = Upstream('')
        self.generation = 0
        self.reloaded_at = 0
        self.state = 'unknown'
        self.since = 0
        self.error = ''
        self.transports = set()
        self.tasks = set()
        self.reload_lock = asyncio.Lock()

    def spawn(self, coroutine):
        task = asyncio.ensure_future(coroutine)
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    def write_status(self):
        line = (f'{os.getpid()} {self.generation} {self.reloaded_at} {self.state} {self.since} '
                f'{self.upstream.label} {self.error}\n')
        tmp = STATUS_FILE + '.tmp'
        with open(tmp, 'w') as file:
            file.write(line)
        os.replace(tmp, STATUS_FILE)

    def set_state(self, state, error=''):
        if state == self.state and error == self.error:
            return
        if state != self.state:
            self.since = int(time.time())
            log(f'upstream {self.upstream.label}: {state}' + (f' ({error})' if error else ''))
        self.state = state
        self.error = error
        self.write_status()

    def drop_connections(self):
        dropped = len(self.transports)
        for transport in list(self.transports):
            transport.abort()
        return dropped

    async def reload(self, drop):
        async with self.reload_lock:
            proc = None
            reason = '.zshrc не прочитан'
            try:
                proc = await asyncio.create_subprocess_exec(
                    UPSTREAM_CMD, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL)
                out, _ = await asyncio.wait_for(proc.communicate(), 10)
                if proc.returncode != 0:
                    raise OSError(f'{os.path.basename(UPSTREAM_CMD)} exited with {proc.returncode}')
                reason = 'не http-прокси'
                upstream = Upstream(out.decode().strip())
            except (OSError, ValueError, asyncio.TimeoutError) as error:
                if proc is not None and proc.returncode is None:
                    proc.kill()
                # The message never echoes the rc line: it may carry credentials.
                log(f'reload failed, keeping {self.upstream.label}: {error}')
                self.reloaded_at = int(time.time())
                self.generation += 1
                self.set_state('error', reason)
                return
            self.upstream = upstream
            self.generation += 1
            self.reloaded_at = int(time.time())
            dropped = self.drop_connections() if drop else 0
            log(f'upstream {upstream.label}' + (f', closed {dropped} open socket(s)' if drop else ''))
            self.state = None
            self.set_state('probing')
            self.spawn(self.check(upstream))

    async def check(self, upstream):
        error = await self.probe(upstream)
        if upstream is self.upstream:
            self.set_state('error' if error else 'ok', error)

    async def probe(self, upstream):
        writer = None
        try:
            if upstream.direct:
                _, writer = await asyncio.wait_for(
                    asyncio.open_connection(*split_host_port(PROBE, 443)), CONNECT_TIMEOUT)
                return ''
            reader, writer = await upstream.open()
            status, _ = await upstream.connect(reader, writer, PROBE)
            return '' if status == 200 else str(status)
        except (asyncio.TimeoutError, asyncio.IncompleteReadError, asyncio.LimitOverrunError, OSError) as error:
            return describe(error)
        finally:
            if writer is not None:
                writer.close()

    async def recheck_loop(self):
        # Sessions sitting idle send nothing, so a failing state would never
        # clear on its own once the upstream recovers.
        while True:
            await asyncio.sleep(RECHECK_SECONDS)
            if self.state == 'error' and not self.reload_lock.locked():
                await self.check(self.upstream)

    async def refuse(self, writer, status, reason):
        body = f'herdr-proxy-relay: {reason}\n'.encode()
        writer.write(
            f'HTTP/1.1 {status} {REASONS[status]}\r\nContent-Type: text/plain; charset=utf-8\r\n'
            f'Content-Length: {len(body)}\r\nConnection: close\r\n\r\n'.encode() + body)
        try:
            await writer.drain()
        except OSError:
            pass

    async def open_tunnel(self, client_writer, upstream, target):
        host, port = split_host_port(target, 443)
        # A target that is down on its own does not make the upstream bad;
        # only the probe host does, since that is the one sessions live on.
        is_probe = f'{host}:{port}' == PROBE
        try:
            if upstream.direct:
                reader, writer = await asyncio.wait_for(asyncio.open_connection(host, port), CONNECT_TIMEOUT)
                client_writer.write(b'HTTP/1.1 200 Connection established\r\n\r\n')
                return reader, writer
            reader, writer = await upstream.open()
        except (asyncio.TimeoutError, OSError) as error:
            if not upstream.direct or is_probe:
                self.report(upstream, describe(error))
            await self.refuse(client_writer, 502, describe(error))
            return None, None
        try:
            status, head = await upstream.connect(reader, writer, target)
        except (asyncio.TimeoutError, asyncio.IncompleteReadError, asyncio.LimitOverrunError, OSError) as error:
            writer.close()
            self.report(upstream, describe(error))
            await self.refuse(client_writer, 502, describe(error))
            return None, None
        client_writer.write(head)
        if status == 200:
            self.report(upstream, '')
            return reader, writer
        if status == 407 or is_probe:
            self.report(upstream, str(status))
        writer.close()
        return None, None

    async def open_forward(self, client_writer, upstream, method, target, version, header_lines):
        headers = [line for line in header_lines
                   if line and line.split(':', 1)[0].strip().lower() not in HOP_BY_HOP]
        if upstream.direct:
            parts = urllib.parse.urlsplit(target)
            if parts.scheme.lower() != 'http' or not parts.hostname:
                await self.refuse(client_writer, 400, 'expected an absolute http:// URL')
                return None, None
            host, port = parts.hostname, parts.port or 80
            path = urllib.parse.urlunsplit(('', '', parts.path or '/', parts.query, ''))
            request_line = f'{method} {path} {version}'
        else:
            host, port = upstream.host, upstream.port
            request_line = f'{method} {target} {version}'
            if upstream.auth:
                headers.append(f'Proxy-Authorization: {upstream.auth}')
        # One request per connection keeps the credential injection trivially
        # right; keep-alive across it would need a full HTTP/1.1 parser.
        headers.append('Connection: close')
        try:
            reader, writer = await asyncio.wait_for(asyncio.open_connection(host, port), CONNECT_TIMEOUT)
        except (asyncio.TimeoutError, OSError) as error:
            if not upstream.direct:
                self.report(upstream, describe(error))
            await self.refuse(client_writer, 502, describe(error))
            return None, None
        writer.write(('\r\n'.join([request_line, *headers]) + '\r\n\r\n').encode('latin-1'))
        return reader, writer

    def report(self, upstream, error):
        if upstream is self.upstream and self.state != 'probing':
            self.set_state('error' if error else 'ok', error)

    async def handle(self, client_reader, client_writer):
        self.transports.add(client_writer.transport)
        upstream_writer = None
        try:
            head = await asyncio.wait_for(client_reader.readuntil(b'\r\n\r\n'), HEAD_TIMEOUT)
            lines = head.decode('latin-1').split('\r\n')
            method, target, version = (lines[0].split(' ', 2) + ['', ''])[:3]
            if not method or not target or not version:
                await self.refuse(client_writer, 400, 'malformed request line')
                return
            upstream = self.upstream
            if method.upper() == 'CONNECT':
                upstream_reader, upstream_writer = await self.open_tunnel(client_writer, upstream, target)
            else:
                upstream_reader, upstream_writer = await self.open_forward(
                    client_writer, upstream, method, target, version, lines[1:])
            if upstream_writer is None:
                return
            self.transports.add(upstream_writer.transport)
            await pipe(client_reader, client_writer, upstream_reader, upstream_writer)
        except (asyncio.TimeoutError, asyncio.IncompleteReadError, asyncio.LimitOverrunError, OSError):
            pass
        finally:
            for writer in (client_writer, upstream_writer):
                if writer is not None:
                    self.transports.discard(writer.transport)
                    writer.close()


async def pipe(a_reader, a_writer, b_reader, b_writer):
    async def copy(reader, writer):
        try:
            while data := await reader.read(CHUNK):
                writer.write(data)
                await writer.drain()
            if writer.can_write_eof() and not writer.is_closing():
                writer.write_eof()
        except OSError:
            a_writer.transport.abort()
            b_writer.transport.abort()

    await asyncio.gather(copy(a_reader, b_writer), copy(b_reader, a_writer))


async def main():
    os.makedirs(STATE_DIR, mode=0o700, exist_ok=True)
    relay = Relay()
    await relay.reload(drop=False)
    server = await asyncio.start_server(
        relay.handle, LISTEN_HOST, LISTEN_PORT, limit=HEAD_LIMIT, reuse_address=True)
    log(f'listening on {LISTEN_HOST}:{LISTEN_PORT}')

    loop = asyncio.get_running_loop()
    stop = loop.create_future()
    loop.add_signal_handler(signal.SIGHUP, lambda: relay.spawn(relay.reload(drop=True)))
    for signum in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(signum, lambda: stop.done() or stop.set_result(None))
    relay.spawn(relay.recheck_loop())

    async with server:
        await stop
    relay.drop_connections()
    try:
        os.unlink(STATUS_FILE)
    except FileNotFoundError:
        pass


if __name__ == '__main__':
    asyncio.run(main())

#!/usr/bin/env python3
"""A clickable button that moves running Claude sessions onto the proxy that
~/.zshrc exports.

A running process cannot be handed a new HTTPS_PROXY, so switching a session is
a restart: /exit, a fresh shell that re-reads ~/.zshrc, claude --resume <id>.
The conversation survives; a turn in progress and its background shells do
not. A busy session is interrupted first — on a dead proxy "working" usually
means retrying anyway. Only sessions whose environment names another proxy are
touched, and one with a draft in its input box is left alone.

herdr forwards a left click to any pane app that turns on mouse reporting, even
when that pane is not focused, so the button is plain terminal output: no herdr
patch and no key binding. Run it as the only pane of its own tab.

Proxy credentials are used for the probe only and never drawn.
"""
import base64
import json
import os
import re
import select
import shlex
import signal
import socket
import ssl
import subprocess
import sys
import termios
import threading
import time
import tty
import urllib.parse

BIN = os.path.dirname(os.path.abspath(__file__))
UPSTREAM_CMD = os.path.join(BIN, 'herdr-proxy-upstream.sh')
HERDR = os.environ.get('HERDR_BIN_PATH') or 'herdr'
OWN_TAB = os.environ.get('HERDR_TAB_ID') or ''
SESSIONS_DIR = os.path.expanduser('~/.claude/sessions')
PROBE_TARGET = ('api.anthropic.com', 443)
# Claude Code's own precedence.
PROXY_VARS = (b'https_proxy', b'HTTPS_PROXY', b'http_proxy', b'HTTP_PROXY')
TAB_LABEL = '⟳ прокси'
REFRESH_SECONDS = 2
REPROBE_SECONDS = 60
# Flags carried over to the resumed session, with their arity. A positional
# argument is a prompt and would be sent again, so it is dropped.
KEEP_FLAGS = {
    '--dangerously-skip-permissions': 0,
    '--chrome': 0,
    '--model': 1,
    '--permission-mode': 1,
    '--add-dir': 1,
    '--settings': 1,
    '--mcp-config': 1,
}
MOUSE = re.compile(rb'\x1b\[<(\d+);(\d+);(\d+)([Mm])')
SGR = re.compile(r'\x1b\[([0-9;]*)m')

RESET, BOLD, DIM = '\x1b[0m', '\x1b[1m', '\x1b[2m'
GREEN, RED, YELLOW = '\x1b[32m', '\x1b[31m', '\x1b[33m'
BUTTON_READY = '\x1b[1;30;42m'
BUTTON_WARN = '\x1b[1;30;43m'
BUTTON_IDLE = '\x1b[2;7m'


def herdr(*args):
    try:
        done = subprocess.run([HERDR, *args], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode != 0:
        return None
    try:
        return json.loads(done.stdout).get('result') or {}
    except ValueError:
        return {}


def read_upstream():
    try:
        done = subprocess.run([UPSTREAM_CMD], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return done.stdout.strip() if done.returncode == 0 else None


def split_url(url):
    return urllib.parse.urlsplit(url if '://' in url else 'http://' + url)


def same_proxy(a, b):
    return (a or '').rstrip('/') == (b or '').rstrip('/')


def label(url):
    if not url:
        return 'напрямую'
    parts = split_url(url)
    try:
        port = parts.port
    except ValueError:
        port = None
    host = parts.hostname or '?'
    return f'{host}:{port}' if port else host


def probe(url):
    """TLS handshake with api.anthropic.com through the proxy; returns (ok, text).

    A local proxy client answers CONNECT with 200 before it reaches its own
    upstream, so the handshake, not the status line, is what proves the path.
    """
    started = time.monotonic()
    tls = ssl.create_default_context()
    try:
        if not url:
            with socket.create_connection(PROBE_TARGET, timeout=8) as sock:
                tls.wrap_socket(sock, server_hostname=PROBE_TARGET[0]).close()
        else:
            parts = split_url(url)
            if parts.scheme != 'http':
                return True, f'{parts.scheme} не проверяю'
            with socket.create_connection((parts.hostname, parts.port or 80), timeout=8) as sock:
                target = '%s:%d' % PROBE_TARGET
                head = f'CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n'
                if parts.username is not None:
                    pair = urllib.parse.unquote(parts.username) + ':' + urllib.parse.unquote(parts.password or '')
                    head += 'Proxy-Authorization: Basic ' + base64.b64encode(pair.encode()).decode() + '\r\n'
                sock.sendall((head + '\r\n').encode())
                reply = b''
                while b'\r\n\r\n' not in reply and len(reply) < 16384:
                    chunk = sock.recv(4096)
                    if not chunk:
                        break
                    reply += chunk
                status = reply.split(b'\r\n', 1)[0].split()
                if len(status) < 2 or b'\r\n\r\n' not in reply:
                    return False, 'обрыв'
                if status[1] != b'200':
                    return False, 'отказ ' + status[1].decode(errors='replace')
                tls.wrap_socket(sock, server_hostname=PROBE_TARGET[0]).close()
    # TimeoutError and gaierror are OSError subclasses, so the order matters.
    except TimeoutError:
        return False, 'таймаут'
    except socket.gaierror:
        return False, 'DNS'
    except ConnectionRefusedError:
        return False, 'порт закрыт'
    except ssl.SSLError:
        return False, 'TLS не прошёл'
    except (OSError, ValueError):
        return False, 'обрыв'
    return True, '%d мс' % ((time.monotonic() - started) * 1000)


def proxy_of(pid):
    try:
        with open(f'/proc/{pid}/environ', 'rb') as f:
            env = dict(item.split(b'=', 1) for item in f.read().split(b'\0') if b'=' in item)
    except OSError:
        return None
    for name in PROXY_VARS:
        if env.get(name):
            return env[name].decode(errors='replace')
    return ''


def parent_of(pid):
    try:
        with open(f'/proc/{pid}/stat') as f:
            return int(f.read().rsplit(')', 1)[1].split()[1])
    except (OSError, IndexError, ValueError):
        return None


def command_of(pid):
    try:
        with open(f'/proc/{pid}/comm') as f:
            return f.read().strip()
    except OSError:
        return None


def alive(pid):
    return os.path.exists(f'/proc/{pid}')


def session_id(pid, argv):
    try:
        with open(os.path.join(SESSIONS_DIR, f'{pid}.json')) as f:
            found = json.load(f).get('sessionId')
        if found:
            return found
    except (OSError, ValueError):
        pass
    for flag in ('--resume', '-r'):
        if flag in argv[:-1]:
            return argv[argv.index(flag) + 1]
    return None


def resume_line(argv, sid):
    kept = []
    i = 1
    while i < len(argv):
        arity = KEEP_FLAGS.get(argv[i])
        if arity is not None:
            kept += argv[i:i + 1 + arity]
            i += arity
        i += 1
    return shlex.join([argv[0], *kept, '--resume', sid])


def claude_in(pane):
    info = (herdr('pane', 'process-info', '--pane', pane) or {}).get('process_info') or {}
    proc = next((p for p in info.get('foreground_processes', []) if p.get('name') == 'claude'), None)
    return proc, info.get('shell_pid')


def collect():
    panes = herdr('pane', 'list')
    if panes is None:
        return None
    tabs = {t['tab_id']: t.get('label') or '' for t in (herdr('tab', 'list') or {}).get('tabs', [])}
    sessions = []
    for pane in panes.get('panes', []):
        if pane.get('agent') != 'claude':
            continue
        proc, shell_pid = claude_in(pane['pane_id'])
        if not proc:
            continue
        status = pane.get('agent_status') or ''
        sessions.append({
            'pane': pane['pane_id'],
            'name': tabs.get(pane.get('tab_id')) or pane.get('terminal_title_stripped') or '',
            'status': status,
            'status_label': (pane.get('state_labels') or {}).get(status, status),
            'pid': proc['pid'],
            'argv': proc.get('argv') or ['claude'],
            'shell_pid': shell_pid,
            'proxy': proxy_of(proc['pid']),
        })
    return sorted(sessions, key=lambda s: s['pane'])


def undimmed(line):
    """The text of a line without its SGR codes and without what is drawn dim."""
    parts, dim, pos = [], False, 0
    for match in SGR.finditer(line):
        if not dim:
            parts.append(line[pos:match.start()])
        params = (match.group(1) or '0').split(';')
        i = 0
        while i < len(params):
            if params[i] in ('38', '48', '58'):
                i += 2 if params[i + 1:i + 2] == ['5'] else 4
            elif params[i] == '2':
                dim = True
            elif params[i] in ('0', '', '22'):
                dim = False
            i += 1
        pos = match.end()
    if not dim:
        parts.append(line[pos:])
    return ''.join(parts)


def draft_in(pane):
    """True when the input box — the last ❯ line on screen — holds typed text.

    An empty box shows a suggestion after ❯ too, but drawn dim.
    """
    try:
        done = subprocess.run([HERDR, 'pane', 'read', pane, '--source', 'visible', '--ansi'],
                              capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return True
    prompts = [line for line in done.stdout.splitlines() if SGR.sub('', line).startswith('❯')]
    return bool(prompts and undimmed(prompts[-1]).lstrip('❯').strip())


def plural(n, one, few, many):
    if n % 10 == 1 and n % 100 != 11:
        return one
    if 2 <= n % 10 <= 4 and not 12 <= n % 100 <= 14:
        return few
    return many


class Button:
    def __init__(self):
        self.lock = threading.Lock()
        self.upstream = None
        self.upstream_read = False
        self.probe = None
        self.probed_at = 0.0
        self.probing = False
        self.sessions = []
        self.jobs = {}
        self.busy = False
        self.confirm = False
        self.message = ''
        self.tab_label = None
        self.rect = None
        self.dirty = threading.Event()

    # ── background refresh ───────────────────────────────────────────────

    def refresh_forever(self):
        while True:
            upstream = read_upstream()
            sessions = collect()
            with self.lock:
                changed = upstream != self.upstream or not self.upstream_read
                self.upstream, self.upstream_read = upstream, True
                if sessions is not None:
                    self.sessions = sessions
                if changed:
                    self.probe, self.confirm = None, False
                stale_probe = time.monotonic() - self.probed_at > REPROBE_SECONDS
                start_probe = upstream is not None and not self.probing and (changed or stale_probe)
                if start_probe:
                    self.probing = True
            if start_probe:
                threading.Thread(target=self.run_probe, args=(upstream,), daemon=True).start()
            self.update_tab()
            self.dirty.set()
            time.sleep(REFRESH_SECONDS)

    def run_probe(self, upstream):
        result = probe(upstream)
        with self.lock:
            self.probing = False
            self.probed_at = time.monotonic()
            if same_proxy(upstream, self.upstream):
                self.probe = result
        self.dirty.set()

    def stale(self):
        return [s for s in self.sessions
                if self.upstream is not None and s['proxy'] is not None
                and not same_proxy(s['proxy'], self.upstream)]

    def update_tab(self):
        if not OWN_TAB:
            return
        with self.lock:
            count = len(self.stale())
        wanted = TAB_LABEL + (f' · {count} на старом' if count else '')
        if wanted != self.tab_label and herdr('tab', 'rename', OWN_TAB, wanted) is not None:
            self.tab_label = wanted

    # ── the switch ───────────────────────────────────────────────────────

    def click(self):
        with self.lock:
            if self.busy or self.upstream is None:
                return
            forced = self.confirm
            self.busy, self.confirm, self.message = True, False, ''
            self.jobs = {}
        threading.Thread(target=self.switch, args=(forced,), daemon=True).start()
        self.dirty.set()

    def switch(self, forced):
        try:
            upstream = read_upstream()
            if upstream is None:
                self.say('.zshrc не прочитан')
                return
            self.say('проверяю прокси…')
            result = probe(upstream)
            with self.lock:
                self.upstream, self.probe, self.probed_at = upstream, result, time.monotonic()
            if not result[0] and not forced:
                with self.lock:
                    self.confirm = True
                self.say('')
                return
            sessions = [s for s in collect() or [] if s['proxy'] is not None
                        and not same_proxy(s['proxy'], upstream)]
            if not sessions:
                self.say('все сессии уже на этом прокси')
                return
            self.say('')
            workers = [threading.Thread(target=self.restart, args=(s, upstream), daemon=True) for s in sessions]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join()
        finally:
            with self.lock:
                self.busy = False
            self.dirty.set()

    def say(self, text):
        with self.lock:
            self.message = text
        self.dirty.set()

    def report(self, pane, state, text):
        with self.lock:
            self.jobs[pane] = (state, text)
        self.dirty.set()

    def restart(self, s, upstream):
        pane, pid = s['pane'], s['pid']
        if s['argv'][1:2] == ['attach']:
            return self.report(pane, 'fail', 'фоновая сессия daemon — не трогаю')
        sid = session_id(pid, s['argv'])
        if not sid:
            return self.report(pane, 'fail', 'не нашёл id сессии')
        if parent_of(pid) != s['shell_pid']:
            return self.report(pane, 'fail', 'claude запущен не из шелла панели')
        if s['status'] in ('working', 'blocked'):
            self.report(pane, 'run', 'прерываю ход…')
            herdr('pane', 'send-keys', pane, 'esc')
            time.sleep(3)
        if draft_in(pane):
            return self.report(pane, 'fail', 'в поле ввода черновик — пропустил')
        shell = command_of(s['shell_pid']) or 'zsh'
        line = resume_line(s['argv'], sid)
        for pause in (3, 6):
            self.report(pane, 'run', 'выхожу…')
            if not self.exit_claude(pane, pid):
                return self.report(pane, 'fail', 'не вышла по /exit')
            self.report(pane, 'run', 'перечитываю .zshrc…')
            time.sleep(2)
            herdr('pane', 'run', pane, f'exec {shell}')
            time.sleep(pause)
            self.report(pane, 'run', 'возобновляю…')
            herdr('pane', 'run', pane, line)
            pid = self.wait_claude(pane, pid)
            if pid is None:
                return self.report(pane, 'fail', 'не поднялась — загляни в панель')
            if same_proxy(proxy_of(pid), upstream):
                return self.report(pane, 'ok', 'переключена')
        self.report(pane, 'fail', 'поднялась на старом прокси')

    @staticmethod
    def exit_claude(pane, pid):
        for second in range(20):
            if not alive(pid):
                return True
            if second in (0, 8):
                herdr('pane', 'run', pane, '/exit')
            time.sleep(1)
        return not alive(pid)

    @staticmethod
    def wait_claude(pane, old_pid):
        for _ in range(40):
            proc, _shell = claude_in(pane)
            if proc and proc['pid'] != old_pid:
                return proc['pid']
            time.sleep(1)
        return None

    # ── drawing ──────────────────────────────────────────────────────────

    def button_face(self):
        stale = len(self.stale())
        if self.busy:
            done = sum(1 for state, _text in self.jobs.values() if state != 'run')
            if self.jobs:
                return BUTTON_IDLE, f'…  Переключаю: {done} из {len(self.jobs)}'
            return BUTTON_IDLE, '…  ' + (self.message or 'Работаю')
        if self.upstream is None:
            return BUTTON_IDLE, '✗  Прокси в .zshrc не прочитан'
        if self.confirm:
            return BUTTON_WARN, '!  Прокси не отвечает. Всё равно переключить?'
        if not stale:
            return BUTTON_IDLE, '✓  Все сессии на прокси из .zshrc'
        return BUTTON_READY, f'⟳  Переключить {stale} {plural(stale, "сессию", "сессии", "сессий")} на него'

    def render(self, cols, rows):
        with self.lock:
            lines = []
            proxy = label(self.upstream) if self.upstream is not None else '—'
            if self.probe is None:
                health = DIM + 'проверяю…' + RESET
            elif self.probe[0]:
                health = GREEN + '● ' + RESET + 'отвечает · ' + self.probe[1]
            else:
                health = RED + '× ' + self.probe[1] + RESET
            lines.append('')
            lines.append(f'  Прокси в ~/.zshrc   {BOLD}{proxy}{RESET}   {health}')
            lines.append('')
            colour, face = self.button_face()
            width = max(20, min(cols - 4, 60))
            top = len(lines) + 1
            for row in range(3):
                text = face if row == 1 else ''
                lines.append('  ' + colour + text.center(width)[:width] + RESET)
            self.rect = (top, top + 2, 3, 2 + width)
            note = self.message if not self.busy else ''
            lines.append('  ' + DIM + note + RESET if note else '')
            lines.append('')
            lines.append(f'  {BOLD}Сессии Claude{RESET}')
            if not self.sessions:
                lines.append('  ' + DIM + 'нет ни одной' + RESET)
            name_width = max(10, min(44, cols - 64))
            for s in self.sessions:
                current = label(s['proxy']) if s['proxy'] is not None else '?'
                state, text = self.jobs.get(s['pane'], (None, ''))
                if state == 'ok':
                    mark, text = GREEN + '✓' + RESET, GREEN + text + RESET
                elif state == 'fail':
                    mark, text = RED + '✗' + RESET, RED + text + RESET
                elif state == 'run':
                    mark, text = YELLOW + '…' + RESET, YELLOW + text + RESET
                elif s['proxy'] is None or self.upstream is None:
                    mark, text = DIM + '?' + RESET, ''
                elif same_proxy(s['proxy'], self.upstream):
                    mark, text = GREEN + '✓' + RESET, DIM + 'уже на нём' + RESET
                else:
                    mark = YELLOW + '↻' + RESET
                    text = 'на старом' + (' · ' + s['status_label'] if s['status_label'] else '')
                pane = s['pane'].split(':')[-1]
                name = s['name'][:name_width]
                lines.append(f'  {mark} {pane:<4} {name:<{name_width}}  {current:<28} {text}')
        return lines[:rows]

    def draw(self, out):
        size = os.get_terminal_size(out.fileno())
        lines = self.render(size.columns, size.lines)
        out.write('\x1b[H' + '\x1b[K\r\n'.join(lines) + '\x1b[K\x1b[J')
        out.flush()

    def hit(self, col, row):
        if self.rect is None:
            return False
        top, bottom, left, right = self.rect
        return top <= row <= bottom and left <= col <= right


def main():
    if not sys.stdin.isatty():
        print('run it in a herdr pane', file=sys.stderr)
        return 1
    fd = sys.stdin.fileno()
    out = sys.stdout
    button = Button()
    signal.signal(signal.SIGWINCH, lambda *_: button.dirty.set())
    saved = termios.tcgetattr(fd)
    tty.setcbreak(fd)
    out.write('\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h')
    threading.Thread(target=button.refresh_forever, daemon=True).start()
    pending = b''
    try:
        while True:
            if button.dirty.is_set():
                button.dirty.clear()
                button.draw(out)
            ready, _w, _x = select.select([fd], [], [], 0.25)
            if not ready:
                continue
            pending += os.read(fd, 1024)
            parsed = 0
            for match in MOUSE.finditer(pending):
                code, col, row, kind = match.groups()
                if kind == b'M' and int(code) == 0 and button.hit(int(col), int(row)):
                    button.click()
                parsed = match.end()
            pending = pending[parsed:]
            # Keep only a report that may still be arriving.
            start = pending.rfind(b'\x1b')
            pending = pending[start:] if start >= 0 and len(pending) - start < 32 else b''
    except KeyboardInterrupt:
        pass
    finally:
        out.write('\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l')
        out.flush()
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
        if OWN_TAB:
            herdr('tab', 'rename', OWN_TAB, TAB_LABEL)
    return 0


if __name__ == '__main__':
    sys.exit(main())

"""Bounded process capture primitives for public-driver evidence."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import threading


class BoundedReader:
    def __init__(self, limit: int) -> None:
        self.data = bytearray()
        self.total = 0
        self.limit = limit

    def record(self, chunk: bytes) -> None:
        self.total += len(chunk)
        if len(self.data) < self.limit:
            self.data.extend(chunk[:self.limit - len(self.data)])

    def drain(self, stream: object) -> None:
        while chunk := stream.read(8192):
            self.record(chunk)


class InterruptReader(BoundedReader):
    def __init__(self, limit: int) -> None:
        super().__init__(limit)
        self.pending = bytearray()
        self.tool_started = threading.Event()
        self.interrupted = threading.Event()

    def drain(self, stream: object) -> None:
        while chunk := stream.read(8192):
            self.record(chunk)
            if self.total <= self.limit:
                self.pending.extend(chunk)
                self.consume_lines()

    def consume_lines(self) -> None:
        while b"\n" in self.pending:
            line, _, remainder = self.pending.partition(b"\n")
            self.pending = bytearray(remainder)
            try:
                event = json.loads(line)
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            message = event.get("message")
            blocks = message.get("content", []) if isinstance(message, dict) else []
            if any(isinstance(block, dict) and block.get("type") == "tool_use" for block in blocks):
                self.tool_started.set()
            if (event.get("type") == "result"
                    and event.get("subtype") == "error_during_execution"
                    and event.get("is_error") is True):
                self.interrupted.set()


class SteerReader(BoundedReader):
    """Recognizes a redacted, replayed steering input after tool use."""

    def __init__(self, limit: int, marker: str) -> None:
        super().__init__(limit)
        self.marker = marker
        self.pending = bytearray()
        self.tool_started = threading.Event()
        self.steer_sent = threading.Event()
        self.steer_echoed = threading.Event()
        self.terminal = threading.Event()

    def drain(self, stream: object) -> None:
        while chunk := stream.read(8192):
            self.record(chunk)
            if self.total <= self.limit:
                self.pending.extend(chunk)
                self.consume_lines()

    def consume_lines(self) -> None:
        while b"\n" in self.pending:
            line, _, remainder = self.pending.partition(b"\n")
            self.pending = bytearray(remainder)
            try:
                event = json.loads(line)
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            message = event.get("message")
            blocks = message.get("content", []) if isinstance(message, dict) else []
            if any(isinstance(block, dict) and block.get("type") == "tool_use" for block in blocks):
                self.tool_started.set()
            if (self.steer_sent.is_set() and event.get("type") == "user"
                    and self.marker.encode() in line):
                self.steer_echoed.set()
            if event.get("type") == "result":
                self.terminal.set()


def _join(readers: list[threading.Thread]) -> None:
    for reader in readers:
        reader.join()


def _check(status: int, stdout: BoundedReader, stderr: BoundedReader, limit: int) -> str:
    if stdout.total > limit or stderr.total > limit:
        raise ValueError("provider output exceeded the public-capture byte limit; no fixture was written")
    if status:
        raise ValueError(f"provider exited {status}; no fixture was written")
    return bytes(stdout.data).decode("utf-8", errors="replace")


def capture(probe: list[str], limit: int, timeout: int) -> str:
    process = subprocess.Popen(probe, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert process.stdout is not None and process.stderr is not None
    stdout, stderr = BoundedReader(limit), BoundedReader(limit)
    readers = [threading.Thread(target=reader.drain, args=(stream,)) for reader, stream in
               ((stdout, process.stdout), (stderr, process.stderr))]
    for reader in readers:
        reader.start()
    try:
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        process.kill()
        process.wait()
        raise ValueError("provider capture timed out; no fixture was written") from error
    _join(readers)
    return _check(status, stdout, stderr, limit)


def capture_interrupt(probe: list[str], limit: int, timeout: int) -> str:
    """Interrupt after a Claude tool-use event, requiring a terminal error result."""
    if os.name == "nt":
        raise ValueError("interrupt capture requires a POSIX process group; no fixture was written")
    process = subprocess.Popen(probe, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True)
    assert process.stdout is not None and process.stderr is not None
    stdout, stderr = InterruptReader(limit), BoundedReader(limit)
    readers = [threading.Thread(target=reader.drain, args=(stream,)) for reader, stream in
               ((stdout, process.stdout), (stderr, process.stderr))]
    for reader in readers:
        reader.start()
    if not stdout.tool_started.wait(timeout):
        process.kill()
        process.wait()
        raise ValueError("Claude tool use was absent before interrupt; no fixture was written")
    os.killpg(process.pid, signal.SIGINT)
    try:
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        raise ValueError("interrupted provider capture timed out; no fixture was written") from error
    _join(readers)
    if not stdout.interrupted.is_set():
        raise ValueError("Claude cancellation result was absent; no fixture was written")
    return _check(status, stdout, stderr, limit)


def capture_steer(probe: list[str], first: str, steer: str, marker: str,
                  limit: int, timeout: int) -> None:
    """Send a second safe user input only after Claude emits a tool-use event."""
    if os.name == "nt":
        raise ValueError("steer capture requires a POSIX process group; no fixture was written")
    process = subprocess.Popen(probe, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    assert process.stdin is not None and process.stdout is not None and process.stderr is not None
    stdout, stderr = SteerReader(limit, marker), BoundedReader(limit)
    readers = [threading.Thread(target=reader.drain, args=(stream,)) for reader, stream in
               ((stdout, process.stdout), (stderr, process.stderr))]
    for reader in readers:
        reader.start()
    try:
        process.stdin.write((json.dumps({"type": "user", "message": {"role": "user", "content": first}}) + "\n").encode())
        process.stdin.flush()
        if not stdout.tool_started.wait(timeout):
            raise ValueError("Claude tool use was absent before steer; no fixture was written")
        stdout.steer_sent.set()
        process.stdin.write((json.dumps({"type": "user", "message": {"role": "user", "content": steer}}) + "\n").encode())
        process.stdin.close()
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        _join(readers)
        raise ValueError("steered provider capture timed out; no fixture was written") from error
    except (OSError, ValueError):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        _join(readers)
        raise
    _join(readers)
    if stdout.total > limit or stderr.total > limit or status:
        raise ValueError("steered provider output was invalid; no fixture was written")
    if not stdout.steer_echoed.is_set() or not stdout.terminal.is_set():
        raise ValueError("Claude steering signal was absent; no fixture was written")

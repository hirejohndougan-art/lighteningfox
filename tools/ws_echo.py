#!/usr/bin/env python3
"""A WebSocket echo server for testing Ironswift (tests/workers.html).

    pip install websockets
    python3 tools/ws_echo.py [port]      # default 8765

Echoes every text or binary message back and answers close frames.
"""
import asyncio, sys

import websockets


async def echo(ws):
    async for msg in ws:
        await ws.send(msg)


async def main(port):
    async with websockets.serve(echo, "127.0.0.1", port):
        print(f"echo server on ws://127.0.0.1:{port}/", flush=True)
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main(int(sys.argv[1]) if len(sys.argv) > 1 else 8765))

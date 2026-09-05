# /// script
# requires-python = ">=3.11"
# dependencies = ["bleak==3.0.1"]
# ///
"""Real Central smoke test; requires a second OS-managed Bluetooth adapter.

Run the Rust example first, then: uv run scripts/verify_gatt.py
Only connects to the target address; never pairs or writes to other devices.
"""
import asyncio
import argparse
from bleak import BleakClient, BleakScanner

SERVICE = "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
RX = "6e400002-b5a3-f393-e0a9-e50e24dcca9e"
TX = "6e400003-b5a3-f393-e0a9-e50e24dcca9e"


async def verify_round(address):
    # Windows can deliver an empty scan response before the name-bearing advertisement.
    device = await BleakScanner.find_device_by_filter(
        lambda device, adv: device.address.lower() == address.lower()
        and adv.local_name == "Rust GATT",
        timeout=15,
    )
    assert device is not None, f"No advertisement from {address}"
    print(f"SCAN PASS: {device.name} ({device.address})", flush=True)
    async with BleakClient(device, timeout=20, winrt={"use_cached_services": False}) as client:
        assert client.services.get_service(SERVICE) is not None
        print("SERVICE DISCOVERY PASS", flush=True)
        value = await client.read_gatt_char(TX, use_cached=False)
        assert value == b"hello from Rust", value
        print(f"READ PASS: {value!r}", flush=True)
        await client.write_gatt_char(RX, b"hello from Central", response=True)
        await client.write_gatt_char(RX, b"write command", response=False)
        print("WRITE REQUEST + COMMAND PASS", flush=True)
        received = asyncio.Event()

        def notification(_, data):
            assert data.startswith(b"tick "), data
            print(f"NOTIFY PASS: {bytes(data)!r}", flush=True)
            received.set()

        await client.start_notify(TX, notification)
        await asyncio.wait_for(received.wait(), timeout=8)
        await client.stop_notify(TX)
        print("UNSUBSCRIBE PASS", flush=True)
    print("DISCONNECT PASS", flush=True)


async def main(address, rounds):
    for round_number in range(1, rounds + 1):
        print(f"ROUND {round_number}/{rounds}", flush=True)
        await verify_round(address)
        if round_number < rounds:
            await asyncio.sleep(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--address", default="08:BE:AC:47:46:F7")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    asyncio.run(main(args.address, args.rounds))

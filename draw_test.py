#!/usr/bin/env python3
"""
TCP Drawing Client for Galactic Unicorn
Sends binary drawing commands over TCP to the Pico
"""

import socket
import struct
import time
import random

# Configuration
PICO_IP = "192.168.1.165"
PICO_PORT = 8080 

# Display dimensions
WIDTH = 53
HEIGHT = 11

# Protocol constants
VERSION = 0x01
CMD_CLEAR = 0x00
CMD_SET_PIXEL = 0x01
RSP_OK = 0x01
RSP_ERROR = 0x00

class DrawingClient:
    def __init__(self, ip: str, port: int):
        self.ip = ip
        self.port = port
        self.sock = None

    def connect(self) -> bool:
        """Connect to the Pico"""
        try:
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.sock.settimeout(5.0)
            print(f"Connecting to {self.ip}:{self.port}...")
            self.sock.connect((self.ip, self.port))
            print("✓ Connected!")
            return True
        except Exception as e:
            print(f"✗ Connection failed: {e}")
            return False

    def send_command(self, data: bytes) -> bool:
        """Send a binary command and wait for response"""
        try:
            # Send command
            self.sock.sendall(data)

            # Wait for response (single byte: 0x01 for success, 0x00 for error)
            response = self.sock.recv(1)
            if len(response) != 1:
                print(f"✗ Invalid response length: {len(response)}")
                return False

            response_byte = response[0]
            if response_byte == RSP_OK:
                return True
            elif response_byte == RSP_ERROR:
                print(f"✗ Server error for command")
                return False
            else:
                print(f"? Unexpected response: 0x{response_byte:02x}")
                return False
        except Exception as e:
            print(f"✗ Send failed: {e}")
            return False

    def clear(self) -> bool:
        """Clear the display"""
        print("Sending CLR command...")
        # Version (1 byte) + Command (1 byte)
        data = struct.pack('BB', VERSION, CMD_CLEAR)
        return self.send_command(data)

    def set_pixel(self, x: int, y: int, r: int, g: int, b: int) -> bool:
        """Set a single pixel (RGB values 0-255)"""
        if not (0 <= x < WIDTH and 0 <= y < HEIGHT):
            print(f"✗ Coordinates out of bounds: ({x}, {y})")
            return False

        if not all(0 <= v <= 255 for v in [r, g, b]):
            print(f"✗ RGB values must be 0-255: ({r}, {g}, {b})")
            return False

        # Version (1) + Command (1) + X (1) + Y (1) + R (1) + G (1) + B (1) = 7 bytes total
        data = struct.pack('BBBBBBB', VERSION, CMD_SET_PIXEL, x, y, r, g, b)
        return self.send_command(data)

    def close(self):
        """Close the connection"""
        if self.sock:
            self.sock.close()
            print("Connection closed")


# Test functions

def test_clear(client: DrawingClient):
    """Test clearing the display"""
    print("\n=== Testing Clear ===")
    client.clear()

def test_single_pixel(client: DrawingClient):
    """Test setting a single pixel"""
    print("\n=== Testing Single Pixel ===")
    x, y = WIDTH // 2, HEIGHT // 2
    print(f"Setting center pixel ({x}, {y}) to white...")
    client.set_pixel(x, y, 255, 255, 255)

def test_line(client: DrawingClient):
    """Test drawing a horizontal line"""
    print("\n=== Testing Horizontal Line ===")
    y = HEIGHT // 2
    print(f"Drawing red line across row {y}...")
    for x in range(WIDTH):
        client.set_pixel(x, y, 255, 0, 0)
        time.sleep(0.01)

def test_border(client: DrawingClient):
    """Test drawing a border"""
    print("\n=== Testing Border ===")
    print("Drawing green border...")

    # Top and bottom
    for x in range(WIDTH):
        client.set_pixel(x, 0, 0, 255, 0)
        client.set_pixel(x, HEIGHT - 1, 0, 255, 0)

    # Left and right
    for y in range(HEIGHT):
        client.set_pixel(0, y, 0, 255, 0)
        client.set_pixel(WIDTH - 1, y, 0, 255, 0)

def test_random_pixels(client: DrawingClient, count: int = 100):
    """Test random pixel updates"""
    print(f"\n=== Testing {count} Random Pixels ===")
    start = time.time()

    success = 0
    for _ in range(count):
        x = random.randint(0, WIDTH - 1)
        y = random.randint(0, HEIGHT - 1)
        r = random.randint(0, 255)
        g = random.randint(0, 255)
        b = random.randint(0, 255)
        if client.set_pixel(x, y, r, g, b):
            success += 1

    elapsed = time.time() - start
    print(f"Sent {success}/{count} pixels in {elapsed:.2f}s ({success/elapsed:.0f} pixels/sec)")

def test_speedtest(client: DrawingClient, count: int = 500):
    """Speed test - send as many pixels as fast as possible"""
    print(f"\n=== Speed Test ({count} pixels) ===")
    print("Sending pixels as fast as possible...")

    start = time.time()
    success = 0
    failed = 0

    for i in range(count):
        x = i % WIDTH
        y = (i // WIDTH) % HEIGHT
        # Alternate colors for visual feedback
        if (i // (WIDTH * HEIGHT)) % 2 == 0:
            r, g, b = 255, 0, 0  # Red
        else:
            r, g, b = 0, 0, 255  # Blue

        if client.set_pixel(x, y, r, g, b):
            success += 1
        else:
            failed += 1

    elapsed = time.time() - start
    print(f"\nResults:")
    print(f"  Total pixels: {count}")
    print(f"  Successful:   {success}")
    print(f"  Failed:       {failed}")
    print(f"  Time:         {elapsed:.2f}s")
    print(f"  Speed:        {success/elapsed:.1f} pixels/sec")
    print(f"  Avg latency:  {elapsed/count*1000:.1f}ms per pixel")

def test_rainbow_sweep(client: DrawingClient):
    """Test sweeping rainbow pattern"""
    print("\n=== Testing Rainbow Sweep ===")
    print("Sweeping rainbow across display...")

    for x in range(WIDTH):
        # HSV to RGB conversion for rainbow effect
        hue = x / WIDTH
        if hue < 1/6:
            r, g, b = 1, hue*6, 0
        elif hue < 2/6:
            r, g, b = (2/6 - hue)*6, 1, 0
        elif hue < 3/6:
            r, g, b = 0, 1, (hue - 2/6)*6
        elif hue < 4/6:
            r, g, b = 0, (4/6 - hue)*6, 1
        elif hue < 5/6:
            r, g, b = (hue - 4/6)*6, 0, 1
        else:
            r, g, b = 1, 0, (1 - hue)*6

        # Convert to 0-255
        r = int(r * 255)
        g = int(g * 255)
        b = int(b * 255)

        # Draw vertical line
        for y in range(HEIGHT):
            client.set_pixel(x, y, r, g, b)

        time.sleep(0.02)

def interactive_mode(client: DrawingClient):
    """Interactive testing mode"""
    print("\n=== Interactive Mode ===")
    print("Commands:")
    print("  pixel X Y       - Set pixel (X, Y) to white")
    print("  pixel X Y R G B - Set pixel (X, Y) to RGB (0-255)")
    print("  clear           - Clear display")
    print("  quit            - Exit")

    while True:
        try:
            cmd = input("\n> ").strip().split()
            if not cmd:
                continue

            if cmd[0].lower() == 'quit':
                break
            elif cmd[0].lower() == 'pixel' and len(cmd) == 3:
                x, y = map(int, cmd[1:])
                if client.set_pixel(x, y, 255, 255, 255):
                    print(f"✓ Set pixel ({x}, {y}) to white")
            elif cmd[0].lower() == 'pixel' and len(cmd) == 6:
                x, y, r, g, b = map(int, cmd[1:])
                if client.set_pixel(x, y, r, g, b):
                    print(f"✓ Set pixel ({x}, {y}) to RGB({r}, {g}, {b})")
            elif cmd[0].lower() == 'clear':
                if client.clear():
                    print("✓ Display cleared")
            else:
                print("Invalid command")
        except KeyboardInterrupt:
            break
        except Exception as e:
            print(f"Error: {e}")


def main():
    print("Galactic Unicorn TCP Drawing Client (Binary Protocol)")
    print(f"Target: {PICO_IP}:{PICO_PORT}")
    print(f"Display: {WIDTH}x{HEIGHT}")

    client = DrawingClient(PICO_IP, PICO_PORT)

    if not client.connect():
        print("\n⚠ Could not connect to Pico. Make sure:")
        print("  1. The Pico is powered on and connected to WiFi")
        print("  2. You're on the same network")
        print("  3. The IP address is correct")
        print("  4. The Draw app is active (press button D)")
        return

    try:
        # Run tests
        test_clear(client)
        time.sleep(1)

        test_single_pixel(client)
        time.sleep(1)

        test_line(client)
        time.sleep(1)

        test_border(client)
        time.sleep(1)

        test_random_pixels(client, 50)
        time.sleep(1)

        test_speedtest(client, 500)
        time.sleep(1)

        test_rainbow_sweep(client)
        time.sleep(1)

        # Enter interactive mode
        # interactive_mode(client)

    finally:
        client.close()
        print("\nDone!")


if __name__ == "__main__":
    main()

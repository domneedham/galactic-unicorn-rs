#!/usr/bin/env python3
"""
Python client for Galactic Unicorn Draw WebSocket API.

Install: pip install websocket-client
Usage: python draw_client.py <hostname> [port]
Example: python draw_client.py 192.168.1.100
"""

import sys
import time
import websocket

# Protocol constants
VERSION = 0x01
CMD_CLEAR = 0x00
CMD_SET_PIXEL = 0x01
CMD_FILL = 0x03
CMD_PING = 0xFE

# Display dimensions
DISPLAY_WIDTH = 53
DISPLAY_HEIGHT = 11


class GalacticUnicornClient:
    """Client for controlling the Galactic Unicorn display via WebSocket."""

    def __init__(self, hostname: str, port: int = 80):
        self.url = f"ws://{hostname}:{port}/draw"
        self.ws = None

    def connect(self):
        """Connect to the WebSocket server."""
        print(f"Connecting to {self.url}...")
        # websocket-client is more lenient and will accept non-standard HTTP responses
        self.ws = websocket.create_connection(
            self.url,
            timeout=10,
            skip_utf8_validation=True
        )
        print("Connected!")

    def disconnect(self):
        """Disconnect from the WebSocket server."""
        if self.ws:
            self.ws.close()
            print("Disconnected")

    def send_command(self, data: bytes):
        """Send a binary command to the device."""
        if self.ws:
            self.ws.send(data, websocket.ABNF.OPCODE_BINARY)

    def clear(self):
        """Clear the display (set all pixels to black)."""
        self.send_command(bytes([VERSION, CMD_CLEAR]))

    def fill(self, r: int, g: int, b: int):
        """Fill the entire display with a solid color."""
        self.send_command(bytes([VERSION, CMD_FILL, r, g, b]))

    def set_pixel(self, x: int, y: int, r: int, g: int, b: int):
        """Set a single pixel to a specific color."""
        if 0 <= x < DISPLAY_WIDTH and 0 <= y < DISPLAY_HEIGHT:
            self.send_command(bytes([VERSION, CMD_SET_PIXEL, x, y, r, g, b]))

    def set_pixels_batch(self, pixels: list):
        """
        Set multiple pixels in a batch.

        Args:
            pixels: List of tuples (x, y, r, g, b)
        """
        commands = []
        for x, y, r, g, b in pixels:
            if 0 <= x < DISPLAY_WIDTH and 0 <= y < DISPLAY_HEIGHT:
                commands.extend([VERSION, CMD_SET_PIXEL, x, y, r, g, b])

        if commands:
            self.send_command(bytes(commands))

    def ping(self):
        """Send a ping to keep the connection alive."""
        self.send_command(bytes([VERSION, CMD_PING]))

    def draw_rectangle(self, x: int, y: int, width: int, height: int,
                       r: int, g: int, b: int):
        """Draw a filled rectangle."""
        pixels = []
        for dy in range(height):
            for dx in range(width):
                px = x + dx
                py = y + dy
                if 0 <= px < DISPLAY_WIDTH and 0 <= py < DISPLAY_HEIGHT:
                    pixels.append((px, py, r, g, b))
        self.set_pixels_batch(pixels)

    def draw_line(self, x0: int, y0: int, x1: int, y1: int,
                  r: int, g: int, b: int):
        """Draw a line using Bresenham's algorithm."""
        pixels = []
        dx = abs(x1 - x0)
        dy = abs(y1 - y0)
        sx = 1 if x0 < x1 else -1
        sy = 1 if y0 < y1 else -1
        err = dx - dy

        x, y = x0, y0
        while True:
            pixels.append((x, y, r, g, b))
            if x == x1 and y == y1:
                break
            e2 = 2 * err
            if e2 > -dy:
                err -= dy
                x += sx
            if e2 < dx:
                err += dx
                y += sy

        self.set_pixels_batch(pixels)


def demo_animation(client: GalacticUnicornClient):
    """Run a demonstration animation."""
    print("\n=== Demo Animation ===\n")

    # Clear display
    print("Clearing display...")
    client.clear()
    time.sleep(0.5)

    # Fill with red
    print("Filling with red...")
    client.fill(255, 0, 0)
    time.sleep(1)

    # Fill with green
    print("Filling with green...")
    client.fill(0, 255, 0)
    time.sleep(1)

    # Fill with blue
    print("Filling with blue...")
    client.fill(0, 0, 255)
    time.sleep(1)

    # Clear and draw some patterns
    print("Drawing patterns...")
    client.clear()
    time.sleep(0.5)

    # Draw colorful rectangles
    print("Drawing rectangles...")
    client.draw_rectangle(0, 0, 10, 5, 255, 0, 0)  # Red
    time.sleep(0.3)
    client.draw_rectangle(15, 0, 10, 5, 0, 255, 0)  # Green
    time.sleep(0.3)
    client.draw_rectangle(30, 0, 10, 5, 0, 0, 255)  # Blue
    time.sleep(0.3)

    # Draw lines
    print("Drawing lines...")
    client.draw_line(0, 7, 52, 7, 255, 255, 0)  # Yellow line
    time.sleep(0.5)
    client.draw_line(0, 9, 52, 9, 255, 0, 255)  # Magenta line
    time.sleep(0.5)

    # Scrolling pixel
    print("Scrolling pixel animation...")
    client.clear()
    for x in range(DISPLAY_WIDTH):
        client.set_pixel(x, DISPLAY_HEIGHT // 2, 0, 255, 255)
        time.sleep(0.05)
        if x > 0:
            client.set_pixel(x - 1, DISPLAY_HEIGHT // 2, 0, 0, 0)

    # Rainbow gradient
    print("Rainbow gradient...")
    client.clear()
    time.sleep(0.3)

    # Send rainbow in chunks to avoid buffer overflow
    chunk_size = 100
    all_pixels = []
    for x in range(DISPLAY_WIDTH):
        # Create a rainbow gradient
        hue = (x / DISPLAY_WIDTH) * 360
        r, g, b = hsv_to_rgb(hue, 1.0, 1.0)
        for y in range(DISPLAY_HEIGHT):
            all_pixels.append((x, y, r, g, b))

    # Send in chunks
    for i in range(0, len(all_pixels), chunk_size):
        chunk = all_pixels[i:i + chunk_size]
        client.set_pixels_batch(chunk)
        time.sleep(0.01)  # Small delay between chunks

    time.sleep(3)

    # Clear at the end
    print("Clearing display...")
    client.clear()

    print("\n=== Demo Complete! ===\n")


def hsv_to_rgb(h: float, s: float, v: float) -> tuple:
    """Convert HSV color to RGB."""
    h = h % 360
    c = v * s
    x = c * (1 - abs((h / 60) % 2 - 1))
    m = v - c

    if 0 <= h < 60:
        r, g, b = c, x, 0
    elif 60 <= h < 120:
        r, g, b = x, c, 0
    elif 120 <= h < 180:
        r, g, b = 0, c, x
    elif 180 <= h < 240:
        r, g, b = 0, x, c
    elif 240 <= h < 300:
        r, g, b = x, 0, c
    else:
        r, g, b = c, 0, x

    return (int((r + m) * 255), int((g + m) * 255), int((b + m) * 255))


def main():
    """Main entry point."""
    if len(sys.argv) < 2:
        print("Usage: python draw_client.py <hostname> [port]")
        print("Example: python draw_client.py 192.168.1.100")
        sys.exit(1)

    hostname = sys.argv[1]
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 80

    client = GalacticUnicornClient(hostname, port)

    try:
        client.connect()
        demo_animation(client)
    except KeyboardInterrupt:
        print("\nInterrupted by user")
    except Exception as e:
        print(f"Error: {e}")
        import traceback
        traceback.print_exc()
    finally:
        client.disconnect()


if __name__ == "__main__":
    main()

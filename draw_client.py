#!/usr/bin/env python3
"""
Python client for Galactic Unicorn Draw WebSocket API.

Install: pip install websocket-client
Usage: python draw_client.py <hostname> [port]
Example: python draw_client.py 192.168.1.100
"""

import sys
import time
import random
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


def perf_individual_pixels(client: GalacticUnicornClient, count: int = 2000):
    """Speed test - send individual pixel commands as fast as possible."""
    print(f"\n=== Perf: Individual Messages ({count} pixels) ===")
    print("Sending one pixel per WebSocket message...")

    client.clear()
    start = time.time()
    success = 0

    for i in range(count):
        x = i % DISPLAY_WIDTH
        y = (i // DISPLAY_WIDTH) % DISPLAY_HEIGHT
        if (i // (DISPLAY_WIDTH * DISPLAY_HEIGHT)) % 2 == 0:
            r, g, b = 255, 0, 0
        else:
            r, g, b = 0, 0, 255

        client.set_pixel(x, y, r, g, b)
        success += 1

    elapsed = time.time() - start

    print(f"  Pixels:     {success}")
    print(f"  Total time: {elapsed*1000:.1f}ms")
    print(f"  Speed:      {success/elapsed:.1f} pixels/sec")


def perf_batched_pixels(client: GalacticUnicornClient, count: int = 5000):
    """Speed test - batch multiple pixel commands into single WebSocket messages."""
    BATCH_SIZE = 50
    print(f"\n=== Perf: Batched Messages ({count} pixels, {BATCH_SIZE}/batch) ===")

    client.clear()
    start = time.time()
    sent = 0
    batch = []

    for i in range(count):
        x = i % DISPLAY_WIDTH
        y = (i // DISPLAY_WIDTH) % DISPLAY_HEIGHT
        if (i // (DISPLAY_WIDTH * DISPLAY_HEIGHT)) % 2 == 0:
            r, g, b = 0, 255, 0
        else:
            r, g, b = 255, 0, 255

        batch.append((x, y, r, g, b))

        if len(batch) >= BATCH_SIZE or i == count - 1:
            client.set_pixels_batch(batch)
            sent += len(batch)
            batch = []

    elapsed = time.time() - start

    print(f"  Pixels:     {sent}")
    print(f"  Total time: {elapsed*1000:.1f}ms")
    print(f"  Speed:      {sent/elapsed:.1f} pixels/sec")


def perf_full_frames(client: GalacticUnicornClient, frames: int = 100):
    """Test full frame updates - send entire display as one batched message per frame."""
    total_pixels = DISPLAY_WIDTH * DISPLAY_HEIGHT
    print(f"\n=== Perf: Full Frames ({frames} frames, {total_pixels} pixels each) ===")

    client.clear()
    start = time.time()

    for frame in range(frames):
        pixels = []
        for y in range(DISPLAY_HEIGHT):
            for x in range(DISPLAY_WIDTH):
                hue = ((x + frame * 3) % DISPLAY_WIDTH) / DISPLAY_WIDTH * 360
                r, g, b = hsv_to_rgb(hue, 1.0, 1.0)
                pixels.append((x, y, r, g, b))

        client.set_pixels_batch(pixels)

    elapsed = time.time() - start
    fps = frames / elapsed
    pixels_per_sec = frames * total_pixels / elapsed

    print(f"  Frames:     {frames}")
    print(f"  Total time: {elapsed*1000:.1f}ms")
    print(f"  FPS:        {fps:.1f}")
    print(f"  Pixels/sec: {pixels_per_sec:.0f}")


def perf_random_pixels(client: GalacticUnicornClient, count: int = 2000):
    """Speed test - random individual pixel writes."""
    print(f"\n=== Perf: Random Pixels ({count}) ===")

    client.clear()
    start = time.time()

    for _ in range(count):
        x = random.randint(0, DISPLAY_WIDTH - 1)
        y = random.randint(0, DISPLAY_HEIGHT - 1)
        r = random.randint(0, 255)
        g = random.randint(0, 255)
        b = random.randint(0, 255)
        client.set_pixel(x, y, r, g, b)

    elapsed = time.time() - start

    print(f"  Pixels:     {count}")
    print(f"  Total time: {elapsed*1000:.1f}ms")
    print(f"  Speed:      {count/elapsed:.1f} pixels/sec")


def perf_fill_cycles(client: GalacticUnicornClient, cycles: int = 30):
    """Speed test - rapid full-screen fill color changes."""
    print(f"\n=== Perf: Fill Cycles ({cycles} fills) ===")

    colors = [
        (255, 0, 0), (0, 255, 0), (0, 0, 255),
        (255, 255, 0), (255, 0, 255), (0, 255, 255),
        (255, 255, 255),
    ]

    start = time.time()

    for i in range(cycles):
        r, g, b = colors[i % len(colors)]
        client.fill(r, g, b)

    elapsed = time.time() - start

    print(f"  Fills:      {cycles}")
    print(f"  Total time: {elapsed*1000:.1f}ms")
    print(f"  Speed:      {cycles/elapsed:.1f} fills/sec")


def perf_bouncing_ball(client: GalacticUnicornClient, duration: float = 10.0):
    """Sustained test - bouncing ball with trail, batched per frame."""
    print(f"\n=== Perf: Bouncing Ball ({duration:.0f}s) ===")

    x, y = DISPLAY_WIDTH / 2.0, DISPLAY_HEIGHT / 2.0
    vx, vy = 1.8, 1.1
    trail = []
    trail_length = 5

    start = time.time()
    frames = 0

    while time.time() - start < duration:
        x += vx
        y += vy

        if x <= 0 or x >= DISPLAY_WIDTH - 1:
            vx = -vx
            x = max(0, min(DISPLAY_WIDTH - 1, x))
        if y <= 0 or y >= DISPLAY_HEIGHT - 1:
            vy = -vy
            y = max(0, min(DISPLAY_HEIGHT - 1, y))

        ix, iy = int(x), int(y)
        trail.append((ix, iy))

        pixels = []

        if len(trail) > trail_length:
            old_x, old_y = trail.pop(0)
            pixels.append((old_x, old_y, 0, 0, 0))

        for i, (tx, ty) in enumerate(trail):
            brightness = int((i + 1) / len(trail) * 255)
            if i == len(trail) - 1:
                pixels.append((tx, ty, 255, 255, 255))
            else:
                pixels.append((tx, ty, 0, 0, brightness))

        client.set_pixels_batch(pixels)
        frames += 1

    elapsed = time.time() - start
    print(f"  Frames:     {frames}")
    print(f"  Total time: {elapsed:.2f}s")
    print(f"  FPS:        {frames/elapsed:.1f}")


def perf_game_of_life(client: GalacticUnicornClient, generations: int = 60):
    """Sustained test - Game of Life, full frame batched per generation."""
    print(f"\n=== Perf: Game of Life ({generations} generations) ===")

    grid = [[random.choice([True, False]) for _ in range(DISPLAY_HEIGHT)]
            for _ in range(DISPLAY_WIDTH)]

    start = time.time()

    for gen in range(generations):
        pixels = []
        for x in range(DISPLAY_WIDTH):
            for y in range(DISPLAY_HEIGHT):
                if grid[x][y]:
                    pixels.append((x, y, 255, 255, 255))
                else:
                    pixels.append((x, y, 0, 0, 0))
        client.set_pixels_batch(pixels)

        new_grid = [[False] * DISPLAY_HEIGHT for _ in range(DISPLAY_WIDTH)]
        for x in range(DISPLAY_WIDTH):
            for y in range(DISPLAY_HEIGHT):
                neighbors = 0
                for dx in [-1, 0, 1]:
                    for dy in [-1, 0, 1]:
                        if dx == 0 and dy == 0:
                            continue
                        nx = (x + dx) % DISPLAY_WIDTH
                        ny = (y + dy) % DISPLAY_HEIGHT
                        if grid[nx][ny]:
                            neighbors += 1
                if grid[x][y]:
                    new_grid[x][y] = neighbors in [2, 3]
                else:
                    new_grid[x][y] = neighbors == 3
        grid = new_grid

    elapsed = time.time() - start
    print(f"  Generations: {generations}")
    print(f"  Total time:  {elapsed:.2f}s")
    print(f"  Gen/sec:     {generations/elapsed:.1f}")
    print(f"  FPS:         {generations/elapsed:.1f}")


def perf_scrolling_rainbow(client: GalacticUnicornClient, duration: float = 10.0):
    """Sustained test - scrolling rainbow, full frame batched."""
    print(f"\n=== Perf: Scrolling Rainbow ({duration:.0f}s) ===")

    start = time.time()
    frames = 0

    while time.time() - start < duration:
        pixels = []
        for y in range(DISPLAY_HEIGHT):
            for x in range(DISPLAY_WIDTH):
                hue = ((x + frames * 2) % DISPLAY_WIDTH) / DISPLAY_WIDTH * 360
                row_shift = (y / DISPLAY_HEIGHT) * 60
                r, g, b = hsv_to_rgb(hue + row_shift, 1.0, 1.0)
                pixels.append((x, y, r, g, b))

        client.set_pixels_batch(pixels)
        frames += 1

    elapsed = time.time() - start
    total_pixels = DISPLAY_WIDTH * DISPLAY_HEIGHT

    print(f"  Frames:     {frames}")
    print(f"  Total time: {elapsed:.2f}s")
    print(f"  FPS:        {frames/elapsed:.1f}")
    print(f"  Pixels/sec: {frames * total_pixels / elapsed:.0f}")


def run_perf_tests(client: GalacticUnicornClient):
    """Run all performance tests."""
    print("\n" + "=" * 50)
    print("  PERFORMANCE TESTS")
    print("=" * 50)

    # Throughput benchmarks
    perf_individual_pixels(client, 2000)
    time.sleep(1)

    perf_batched_pixels(client, 5000)
    time.sleep(1)

    perf_fill_cycles(client, 30)
    time.sleep(1)

    perf_full_frames(client, 100)
    time.sleep(1)

    perf_random_pixels(client, 2000)
    time.sleep(1)

    # Sustained visual tests
    client.clear()
    perf_scrolling_rainbow(client, 10.0)
    time.sleep(1)

    client.clear()
    perf_bouncing_ball(client, 10.0)
    time.sleep(1)

    client.clear()
    perf_game_of_life(client, 60)

    client.clear()
    print("\n" + "=" * 50)
    print("  PERFORMANCE TESTS COMPLETE")
    print("=" * 50)


def main():
    """Main entry point."""
    if len(sys.argv) < 2:
        print("Usage: python draw_client.py <hostname> [port] [--perf]")
        print("Example: python draw_client.py 192.168.1.100")
        print("         python draw_client.py 192.168.1.100 --perf")
        sys.exit(1)

    hostname = sys.argv[1]
    port = 80
    run_perf = False

    for arg in sys.argv[2:]:
        if arg == "--perf":
            run_perf = True
        else:
            try:
                port = int(arg)
            except ValueError:
                pass

    client = GalacticUnicornClient(hostname, port)

    try:
        client.connect()
        if run_perf:
            run_perf_tests(client)
        else:
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

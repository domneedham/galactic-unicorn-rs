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

            return True
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

def test_finger_drawing(client: DrawingClient, duration: int = 10):
    """Simulate finger drawing - random walk path"""
    print(f"\n=== Finger Drawing Simulation ({duration}s) ===")
    print("Simulating continuous drawing with random walk...")

    # Start in middle
    x, y = WIDTH // 2, HEIGHT // 2

    start = time.time()
    pixels_drawn = 0

    while time.time() - start < duration:
        # Draw current position
        # Use varying colors based on position for visual interest
        r = int((x / WIDTH) * 255)
        g = int((y / HEIGHT) * 255)
        b = 128
        client.set_pixel(x, y, r, g, b)
        pixels_drawn += 1

        # Random walk
        dx = random.choice([-1, 0, 1])
        dy = random.choice([-1, 0, 1])
        x = max(0, min(WIDTH - 1, x + dx))
        y = max(0, min(HEIGHT - 1, y + dy))

        time.sleep(0.05)  # 20 FPS drawing speed

    elapsed = time.time() - start
    print(f"Drew {pixels_drawn} pixels in {elapsed:.2f}s ({pixels_drawn/elapsed:.1f} pixels/sec)")

def test_snake_game(client: DrawingClient, duration: int = 15):
    """Simulate a snake game"""
    print(f"\n=== Snake Game Simulation ({duration}s) ===")
    print("Running snake game...")

    # Initial snake (3 segments)
    snake = [(WIDTH // 2, HEIGHT // 2), (WIDTH // 2 - 1, HEIGHT // 2), (WIDTH // 2 - 2, HEIGHT // 2)]
    direction = (1, 0)  # Moving right

    # Food
    food = (random.randint(0, WIDTH - 1), random.randint(0, HEIGHT - 1))

    start = time.time()
    frames = 0

    while time.time() - start < duration:
        # Calculate new head position
        head_x, head_y = snake[0]
        new_head = ((head_x + direction[0]) % WIDTH, (head_y + direction[1]) % HEIGHT)

        # Check if food eaten
        ate_food = new_head == food

        # Move snake
        snake.insert(0, new_head)
        if not ate_food:
            tail = snake.pop()
            # Clear tail
            client.set_pixel(tail[0], tail[1], 0, 0, 0)
        else:
            # Spawn new food
            food = (random.randint(0, WIDTH - 1), random.randint(0, HEIGHT - 1))

        # Draw snake head (green)
        client.set_pixel(snake[0][0], snake[0][1], 0, 255, 0)

        # Draw body (darker green)
        for x, y in snake[1:]:
            client.set_pixel(x, y, 0, 128, 0)

        # Draw food (red)
        client.set_pixel(food[0], food[1], 255, 0, 0)

        # Randomly change direction
        if random.random() < 0.2:
            direction = random.choice([(1, 0), (-1, 0), (0, 1), (0, -1)])

        frames += 1
        time.sleep(0.15)  # ~6-7 FPS

    elapsed = time.time() - start
    print(f"Ran {frames} frames in {elapsed:.2f}s ({frames/elapsed:.1f} FPS)")
    print(f"Final snake length: {len(snake)}")

def test_game_of_life(client: DrawingClient, generations: int = 50):
    """Conway's Game of Life simulation"""
    print(f"\n=== Game of Life ({generations} generations) ===")
    print("Running Conway's Game of Life...")

    # Initialize random grid
    grid = [[random.choice([True, False]) for _ in range(HEIGHT)] for _ in range(WIDTH)]

    start = time.time()

    for gen in range(generations):
        # Draw current generation
        for x in range(WIDTH):
            for y in range(HEIGHT):
                if grid[x][y]:
                    # Living cells in white
                    client.set_pixel(x, y, 255, 255, 255)
                else:
                    # Dead cells in black
                    client.set_pixel(x, y, 0, 0, 0)

        # Calculate next generation
        new_grid = [[False] * HEIGHT for _ in range(WIDTH)]
        for x in range(WIDTH):
            for y in range(HEIGHT):
                # Count living neighbors
                neighbors = 0
                for dx in [-1, 0, 1]:
                    for dy in [-1, 0, 1]:
                        if dx == 0 and dy == 0:
                            continue
                        nx, ny = (x + dx) % WIDTH, (y + dy) % HEIGHT
                        if grid[nx][ny]:
                            neighbors += 1

                # Apply rules
                if grid[x][y]:  # Cell is alive
                    new_grid[x][y] = neighbors in [2, 3]
                else:  # Cell is dead
                    new_grid[x][y] = neighbors == 3

        grid = new_grid
        time.sleep(0.2)  # 5 FPS

    elapsed = time.time() - start
    print(f"Ran {generations} generations in {elapsed:.2f}s ({generations/elapsed:.1f} gen/sec)")

def test_bouncing_ball(client: DrawingClient, duration: int = 15):
    """Bouncing ball / DVD screensaver simulation"""
    print(f"\n=== Bouncing Ball ({duration}s) ===")
    print("Running bouncing ball simulation...")

    # Ball state
    x, y = WIDTH / 2, HEIGHT / 2
    vx, vy = 1.5, 0.8
    trail_length = 3
    trail = []

    start = time.time()
    frames = 0

    while time.time() - start < duration:
        # Update position
        x += vx
        y += vy

        # Bounce off edges
        if x <= 0 or x >= WIDTH - 1:
            vx = -vx
            x = max(0, min(WIDTH - 1, x))
        if y <= 0 or y >= HEIGHT - 1:
            vy = -vy
            y = max(0, min(HEIGHT - 1, y))

        # Integer position for drawing
        ix, iy = int(x), int(y)

        # Add to trail
        trail.append((ix, iy))
        if len(trail) > trail_length:
            # Clear oldest trail pixel
            old_x, old_y = trail.pop(0)
            client.set_pixel(old_x, old_y, 0, 0, 0)

        # Draw trail (fading)
        for i, (tx, ty) in enumerate(trail):
            brightness = int((i + 1) / len(trail) * 255)
            if i == len(trail) - 1:
                # Ball is bright white
                client.set_pixel(tx, ty, 255, 255, 255)
            else:
                # Trail fades from blue to black
                client.set_pixel(tx, ty, 0, 0, brightness)

        frames += 1
        time.sleep(0.0167)  # ~60 FPS (1/60 second)

    elapsed = time.time() - start
    print(f"Ran {frames} frames in {elapsed:.2f}s ({frames/elapsed:.1f} FPS)")

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
        # Run basic tests
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

        # Run fun demos
        test_clear(client)
        time.sleep(0.5)

        test_finger_drawing(client, duration=10)
        time.sleep(1)

        test_clear(client)
        time.sleep(0.5)

        test_bouncing_ball(client, duration=15)
        time.sleep(1)

        test_clear(client)
        time.sleep(0.5)

        test_snake_game(client, duration=15)
        time.sleep(1)

        test_clear(client)
        time.sleep(0.5)

        test_game_of_life(client, generations=50)
        time.sleep(1)

        # Enter interactive mode
        # interactive_mode(client)

    finally:
        client.close()
        print("\nDone!")


if __name__ == "__main__":
    main()

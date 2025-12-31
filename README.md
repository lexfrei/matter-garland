# Matter Garland

Matter/Thread Christmas garland controller for ESP32-C6.

## Hardware

- ESP32-C6-DevKitC-1
- 2N7000 N-channel MOSFET
- 10kΩ pull-down resistor
- LED garland (3V)

### Wiring

```
GPIO18 ──┬── Gate (middle)
         │
        10kΩ
         │
GND ─────┴── Source (left)

Drain (right) ── Garland (-)
3.3V ─────────── Garland (+)
```

## Build & Flash

```bash
cargo +nightly espflash flash --target riscv32imac-esp-espidf --port /dev/cu.usbmodem101 --monitor
```

## Commission

1. Scan QR code from serial monitor
2. Add to Apple Home / Google Home / Home Assistant

## License

MIT

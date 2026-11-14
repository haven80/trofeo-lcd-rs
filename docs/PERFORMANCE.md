# Performance

[← Back to the README](../README.md) · [All the docs](./README.md)

Measured on a Windows PC (12 logical processors), running `trofeo_lcd` at
default settings (idle FPS 2 / active 15, DeepCool enabled), before the pixel
game existed:

| Metric | Measured |
|---|---|
| RAM | ~13 MB working set (stable, no growth) |

It stays this light thanks to adaptive FPS (JPEG encode + USB transfer only
happen while there is audio) and the CPU optimizations in `src/main.rs` /
`src/lib.rs`.

The **pixel game** (on by default) trades some of that for animation: while it
is on screen the idle screen runs at the active FPS (15) instead of the idle
one (2). Drawing the game itself costs well under a millisecond per frame; the
extra load is JPEG-encoding and sending more frames. Set `pixel_game = false`
to get the minimal-load behavior measured above. These figures have not been
re-measured with the game on.

An **animated background effect** works the same way: building the picture takes about a
millisecond (matrix and rain a little less; the aquarium about 5 ms, the network 1-2 ms and the globe about 3 ms), and the cost that matters is
the extra frames. It uses its own, low `effect_fps` (default 8) while it moves instead of the
idle 2 fps, so it sits between the idle screen and the game; lower `effect_fps` or set
`effect_speed = 0` (a still picture, idle FPS unchanged) to make it lighter still. A **solid
color** background costs nothing extra.

The **digital frame** idles at 2 frames a second (the collage is kept as a finished picture and
copied to the screen) and asks for 20 only while a picture is changing, for about a second and a
half every `frame_interval` seconds. Decoding and scaling a photo happens on a background thread,
once per change. The **tile colors from the presets** need 5 frames a second.

The **settings editor** is not part of this: it is a separate program, and it only needs to be
open while you change settings. Its preview starts `trofeo_lcd` for a moment to draw each frame
and exits; it never touches the LCD, the audio or the sensors, so it can be used while the
display is running.

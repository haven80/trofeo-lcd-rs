# trofeo-lcd-rs v3.1.0

The statistics of your **Pi-hole** (version 6) on the LCD. Optional: nothing changes unless you set `pihole_url`. This changelog also covers 3.0.1 (the second-monitor panel no longer goes to standby on a still screen, thanks to @WesleyFilips) and 3.0.0 (the music spectrum in your layouts, flowing colours on the classic screens) for anyone who has not published them yet.

## Added in 3.1.0

- **Pi-hole statistics.** Queries in the last 24 hours, blocked, the share blocked, the domains on the block lists, and cached, forwarded and clients. Set it up in `trofeo.conf` (or in the settings editor, new *Pi-hole* section; the password is shown as dots):

  ```ini
  pihole_url      = http://192.168.178.10:8080
  pihole_password = your-app-password
  pihole_interval = 15     # seconds, 5-3600
  pihole_timeout  = 5      # seconds, 1-60
  layout          = pihole
  ```

  Address, password and times are settings, not code. All four need a restart. An app password (Pi-hole: *Settings → Web interface / API → Configure app password*) is best; a password with a `#` goes in double quotes.
- **One login, the session is kept.** The program logs in once (`POST /api/auth`), keeps the session id and sends it as `X-FTL-SID` with every `GET /api/stats/summary`. Only a `401` (the session has expired) makes it log in again, once, and ask again, so the sessions Pi-hole allows are never used up. After a refused login, or *too many requests*, it waits at least a minute. It all runs on a thread of its own: a slow or switched-off Pi-hole never slows the screen, the last figures stay with a status (`ONLINE`, `OFFLINE`, `LOGIN?`), and the console tells you what is wrong when something changes.
- **Seven designs**, each with a portrait version: `pihole` (a big ring, the figures in a row), `pihole-cards` (four cards), `pihole-ring` (one big ring in the middle, flowing colours), `pihole-console` (a pixel-font readout), `pihole-minimal` (the share, huge and light), `pihole-light` (a light page with a red accent) and `pihole-flow` (three wide bars with flowing colours: blocked, cache, forwarded). 33 designs in all. They rotate with the other screens (`layout = pihole:10, default`) and can be copied in the layout builder.
- **A `pihole` panel** in `deck`, `cards`, `orbit` and `console`, and a `pihole` tile in `tiles` (`deck = cpu, gpu, pihole, clock`).
- **In your own layouts**: the texts `{pihole_queries}` `{pihole_blocked}` `{pihole_allowed}` `{pihole_percent}` `{pihole_cached}` `{pihole_forwarded}` `{pihole_cached_percent}` `{pihole_forwarded_percent}` `{pihole_domains}` `{pihole_clients}` `{pihole_status}`, and the bar and ring values `pihole_percent`, `pihole_cached_percent` and `pihole_forwarded_percent`, also in the layout builder.
- Documentation: a *Pi-hole* section in the options guide, the README and `docs/LAYOUTS.md` show the designs, new pictures in `img/`.
- New unit tests, among them a tiny fake Pi-hole on localhost that checks: one login for several refreshes, the session id sent, exactly one new login after a `401`, a refused password reported and not repeated, no login without a password, and a Pi-hole that is not there.

## Notes

- Only Pi-hole **6** has this API (version 5 does not). With Docker use the port you published.
- The password stays in `trofeo.conf` in clear text, like the other settings. Do not post the file in an issue without taking it out.
- The pictures of the settings editor and `--preview-png` show sample figures, not your Pi-hole's.
- Plain `http://` and `https://` with a certificate your computer trusts work; a self-signed certificate is refused.

## Also in this package

- **3.0.0** — a `spectrum` element for free layouts (row, column, circle; left / right channel; bass / mid / treble; mirrored), four designs (`pulse`, `radial`, `stereo`, `bands`), `spectrum_gain`, `spectrum_compress`, `spectrum_auto_gain`, `audio_device`, and flowing colours on the classic screens (`dash_flow`, `tiles_flow`).
- **2.25.0 — 2.19.0** — see [the history](docs/HISTORY.md).

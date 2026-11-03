#!/usr/bin/env python3
"""Writes docs/assets/architecture-{light,dark}.svg, the animated diagram in the README.

Each scene is boxes, wires and packets. A packet travels a list of box ids; the script turns
that into CSS keyframes, so no coordinate of the animation is written by hand. The scenes play
one after another in a loop. Run it with `just readme-svg` after changing a scene.
"""

from pathlib import Path

W, H = 880, 320
BOX_W, BOX_H = 180, 56
FADE = 0.4
PALETTES = {
    "light": {"bg": "#f6f4ef", "ink": "#1d1d1b", "muted": "#77736b", "line": "#b9b4a8",
              "box": "#ffffff", "ok": "#15803d", "bad": "#b91c1c", "ack": "#1d4ed8"},
    "dark": {"bg": "#16171a", "ink": "#e8e6e1", "muted": "#8a877f", "line": "#4a4943",
             "box": "#1f2024", "ok": "#4ade80", "bad": "#f87171", "ack": "#60a5fa"},
}

# box: id -> (center x, center y, name, role)
# wire: (from id, to id, label)
# packet: (start second within the scene, box ids travelled, kind: ok | bad | ack)
SCENES = [
    {
        "title": "bridge two clusters",
        "caption": "subscribe on one server, publish on another",
        "seconds": 6,
        "boxes": {
            "src": (130, 170, "nats edge", "eu-west"),
            "naps": (440, 170, "naps", "route: telemetry"),
            "dst": (750, 170, "nats core", "us-east"),
        },
        "wires": [("src", "naps", "telemetry.>"), ("naps", "dst", "telemetry.>")],
        "packets": [(0.3, ["src", "naps", "dst"], "ok"),
                    (1.5, ["src", "naps", "dst"], "ok"),
                    (2.7, ["src", "naps", "dst"], "ok")],
    },
    {
        "title": "filter and transform",
        "caption": "orders.ts drops what is not confirmed and renames the rest",
        "seconds": 7,
        "boxes": {
            "src": (130, 170, "nats", "source"),
            "naps": (440, 170, "naps", "processor: orders.ts"),
            "dst": (750, 170, "nats", "destination"),
        },
        "wires": [("src", "naps", "orders.>"), ("naps", "dst", "orders.confirmed")],
        "packets": [(0.3, ["src", "naps", "dst"], "ok"),
                    (1.5, ["src", "naps"], "bad"),
                    (2.7, ["src", "naps", "dst"], "ok"),
                    (3.9, ["src", "naps"], "bad")],
    },
    {
        "title": "fan-out",
        "caption": "the processor returns a list: one message in, three out",
        "seconds": 6,
        "boxes": {
            "src": (130, 170, "nats", "orders.created"),
            "naps": (440, 170, "naps", "processor: fanout.lua"),
            "a": (750, 90, "billing", "billing.orders"),
            "b": (750, 170, "shipping", "shipping.orders"),
            "c": (750, 250, "audit", "audit.orders"),
        },
        "wires": [("src", "naps", ""), ("naps", "a", ""), ("naps", "b", ""), ("naps", "c", "")],
        "packets": [(0.3, ["src", "naps"], "ok"),
                    (1.2, ["naps", "a"], "ok"),
                    (1.2, ["naps", "b"], "ok"),
                    (1.2, ["naps", "c"], "ok")],
    },
    {
        "title": "jetstream, at least once",
        "caption": "ack after publish; on error nak, and the stream redelivers",
        "seconds": 10,
        "boxes": {
            "src": (130, 170, "stream ORDERS", "consumer: naps-orders"),
            "naps": (440, 170, "naps", "pull consumer"),
            "dst": (750, 170, "stream AUDIT", "to.mode: jetstream"),
        },
        "wires": [("src", "naps", "pull"), ("naps", "dst", "publish")],
        "packets": [(0.3, ["src", "naps", "dst"], "ok"),
                    (3.1, ["naps", "src"], "ack"),
                    (4.0, ["src", "naps"], "bad"),
                    (4.9, ["naps", "src"], "bad"),
                    (5.8, ["src", "naps", "dst"], "ok"),
                    (8.6, ["naps", "src"], "ack")],
    },
    {
        "title": "kubernetes",
        "caption": "helm install; replicas share the subject through a queue group",
        "seconds": 7,
        "boxes": {
            "src": (130, 170, "nats", "orders.>"),
            "p1": (440, 100, "naps-0", "pod"),
            "p2": (440, 170, "naps-1", "pod"),
            "p3": (440, 240, "naps-2", "pod"),
            "dst": (750, 170, "nats", "destination"),
        },
        "frame": ("p1", "p3", "queue group naps-orders"),
        "wires": [("src", "p1", ""), ("src", "p2", ""), ("src", "p3", ""),
                  ("p1", "dst", ""), ("p2", "dst", ""), ("p3", "dst", "")],
        "packets": [(0.3, ["src", "p1", "dst"], "ok"),
                    (1.1, ["src", "p2", "dst"], "ok"),
                    (1.9, ["src", "p3", "dst"], "ok"),
                    (2.7, ["src", "p1", "dst"], "ok"),
                    (3.5, ["src", "p2", "dst"], "ok")],
    },
]
SPEED = 160  # pixels per second
TOTAL = sum(s["seconds"] for s in SCENES)


def pct(t):
    return f"{100 * t / TOTAL:.3f}%"


def edge_point(box, toward):
    """Where the line from box's center toward the point `toward` leaves the box."""
    x, y = box[0], box[1]
    dx, dy = toward[0] - x, toward[1] - y
    sx = (BOX_W / 2) / abs(dx) if dx else float("inf")
    sy = (BOX_H / 2) / abs(dy) if dy else float("inf")
    s = min(sx, sy)
    return x + dx * s, y + dy * s


def wire_ends(boxes, a, b):
    return edge_point(boxes[a], boxes[b]), edge_point(boxes[b], boxes[a])


def scene_keyframes(name, start, seconds):
    end = start + seconds
    stops = [(0, 0), (start, 0), (start + FADE, 1), (end - FADE, 1), (end, 0), (TOTAL, 0)]
    frames = " ".join(f"{pct(t)}{{opacity:{o}}}" for t, o in dedupe(stops))
    return f"@keyframes {name}{{{frames}}}"


def packet_keyframes(name, boxes, start, path):
    points = []
    for a, b in zip(path, path[1:]):
        src, dst = wire_ends(boxes, a, b)
        points += [src, dst]
    t = start
    stops = [(0, points[0], 0), (t, points[0], 0), (t + 0.05, points[0], 1)]
    t += 0.05
    for p, q in zip(points, points[1:]):
        t += ((q[0] - p[0]) ** 2 + (q[1] - p[1]) ** 2) ** 0.5 / SPEED
        stops.append((t, q, 1))
    last = points[-1]
    stops += [(t + 0.25, last, 0), (TOTAL, last, 0)]
    frames = " ".join(
        f"{pct(t)}{{transform:translate({x:.1f}px,{y:.1f}px);opacity:{o}}}"
        for t, (x, y), o in dedupe(stops)
    )
    return f"@keyframes {name}{{{frames}}}"


def dedupe(stops):
    """Keeps the last stop when two land on the same time."""
    out = []
    for stop in stops:
        if out and abs(out[-1][0] - stop[0]) < 1e-9:
            out[-1] = stop
        else:
            out.append(stop)
    return out


def legend():
    items = [("ok", "message"), ("bad", "dropped / nak"), ("ack", "ack")]
    out, x = [], W - 330
    for kind, text in items:
        out.append(f'<circle class="{kind}" cx="{x}" cy="{H - 28}" r="5"/>')
        out.append(f'<text class="caption" x="{x + 10}" y="{H - 24}">{text}</text>')
        x += 40 + 8 * len(text)
    return out


def render(palette):
    c = PALETTES[palette]
    css = [
        f"svg{{background:{c['bg']}}}",
        "text{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}",
        f".title{{fill:{c['ink']};font-size:20px;font-weight:600}}",
        f".caption,.role,.label,.count{{fill:{c['muted']};font-size:12px}}",
        f".name{{fill:{c['ink']};font-size:15px;font-weight:600}}",
        f".box{{fill:{c['box']};stroke:{c['ink']};stroke-width:1.5}}",
        f".frame{{fill:none;stroke:{c['muted']};stroke-width:1;stroke-dasharray:4 4}}",
        f".wire{{stroke:{c['line']};stroke-width:1.5;fill:none}}",
        f".ok{{fill:{c['ok']}}} .bad{{fill:{c['bad']}}} .ack{{fill:{c['ack']}}}",
        ".scene,.packet{animation-duration:%ss;animation-iteration-count:infinite;"
        "animation-timing-function:linear}" % TOTAL,
        ".scene{opacity:0}",
        ".packet{opacity:0}",
    ]
    body = []
    start = 0
    for i, scene in enumerate(SCENES):
        sid = f"s{i}"
        css.append(scene_keyframes(sid, start, scene["seconds"]))
        css.append(f"#{sid}{{animation-name:{sid}}}")
        boxes = scene["boxes"]
        g = [f'<g class="scene" id="{sid}">',
             f'<text class="count" x="40" y="40">{i + 1}/{len(SCENES)}</text>',
             f'<text class="title" x="80" y="42">{scene["title"]}</text>',
             f'<text class="caption" x="40" y="{H - 24}">{scene["caption"]}</text>']
        if "frame" in scene:
            top, bottom, label = scene["frame"]
            x0 = boxes[top][0] - BOX_W / 2 - 14
            y0 = boxes[top][1] - BOX_H / 2 - 22
            y1 = boxes[bottom][1] + BOX_H / 2 + 10
            g.append(f'<rect class="frame" x="{x0}" y="{y0}" width="{BOX_W + 28}" '
                     f'height="{y1 - y0}" rx="6"/>')
            g.append(f'<text class="label" x="{x0 + 6}" y="{y0 + 14}">{label}</text>')
        for a, b, label in scene["wires"]:
            (x1, y1), (x2, y2) = wire_ends(boxes, a, b)
            g.append(f'<line class="wire" x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" '
                     f'y2="{y2:.1f}" marker-end="url(#head)"/>')
            if label:
                g.append(f'<text class="label" x="{(x1 + x2) / 2:.1f}" y="{(y1 + y2) / 2 - 10:.1f}" '
                         f'text-anchor="middle">{label}</text>')
        for j, (at, path, kind) in enumerate(scene["packets"]):
            pid = f"{sid}p{j}"
            css.append(packet_keyframes(pid, boxes, start + at, path))
            css.append(f"#{pid}{{animation-name:{pid}}}")
            g.append(f'<circle class="packet {kind}" id="{pid}" r="6"/>')
        for x, y, name, role in boxes.values():
            g.append(f'<rect class="box" x="{x - BOX_W / 2}" y="{y - BOX_H / 2}" '
                     f'width="{BOX_W}" height="{BOX_H}" rx="4"/>')
            g.append(f'<text class="name" x="{x}" y="{y - 3}" text-anchor="middle">{name}</text>')
            g.append(f'<text class="role" x="{x}" y="{y + 15}" text-anchor="middle">{role}</text>')
        g.append("</g>")
        body += g
        start += scene["seconds"]
    return "\n".join([
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        f'viewBox="0 0 {W} {H}" role="img" aria-label="naps architecture examples">',
        "<defs>",
        '<marker id="head" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
        'markerHeight="7" orient="auto">',
        f'<path d="M0,0 L10,5 L0,10 z" fill="{c["line"]}"/>',
        "</marker>",
        "</defs>",
        "<style>",
        *css,
        "</style>",
        f'<rect width="{W}" height="{H}" fill="{c["bg"]}"/>',
        *legend(),
        *body,
        "</svg>",
        "",
    ])


def main():
    out = Path(__file__).resolve().parent.parent / "docs" / "assets"
    out.mkdir(parents=True, exist_ok=True)
    for palette in PALETTES:
        (out / f"architecture-{palette}.svg").write_text(render(palette))


if __name__ == "__main__":
    main()

import { degrees, lunarPass, project } from './sky-model.mjs';

const turn = Math.PI * 2;

function circle(ctx, x, y, radius) {
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, turn);
}

function direction({ altitude, azimuth }) {
  const z = altitude * degrees;
  const a = azimuth * degrees;
  return [Math.cos(z) * Math.cos(a), Math.cos(z) * Math.sin(a), Math.sin(z)];
}

function moonMarker(ctx, moon, sun, point, x, y, radius) {
  const moonVector = direction(moon);
  const sunVector = direction(sun);
  const dot = moonVector.reduce((sum, value, index) => sum + value * sunVector[index], 0);
  const tangent = moonVector.map((value, index) => sunVector[index] - value * dot);
  const [north, east, up] = moonVector.map((value, index) => value + tangent[index] * 0.01);
  const [sunX, sunY] = point(
    Math.atan2(up, Math.hypot(north, east)) / degrees,
    Math.atan2(east, north) / degrees,
  );

  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(Math.atan2(sunY - y, sunX - x));
  circle(ctx, 0, 0, radius);
  ctx.fillStyle = '#1c1c1c';
  ctx.fill();
  const terminator = (1 - 2 * Math.max(0, Math.min(1, moon.phase_fraction))) * radius;
  ctx.beginPath();
  ctx.arc(0, 0, radius, -Math.PI / 2, Math.PI / 2);
  if (Math.abs(terminator) < 1e-6) {
    ctx.lineTo(0, -radius);
  } else {
    ctx.ellipse(0, 0, Math.abs(terminator), radius, 0, Math.PI / 2, -Math.PI / 2, terminator > 0);
  }
  ctx.closePath();
  ctx.fillStyle = '#efefef';
  ctx.fill();
  circle(ctx, 0, 0, radius);
  ctx.strokeStyle = '#929292';
  ctx.lineWidth = 0.7;
  ctx.stroke();
  ctx.restore();
}

function atmosphere(ctx, width, height, sun, point) {
  const light = Math.max(0, Math.min(1, (sun.altitude + 14) / 28));
  const dusk = Math.max(0, 1 - Math.abs(sun.altitude + 3) / 14);
  const [x, y] = point(Math.max(0, sun.altitude), sun.azimuth);
  const distance = Math.hypot(width, height);
  const sky = ctx.createLinearGradient(0, 0, 0, height);
  sky.addColorStop(0, `hsl(${218 - 14 * light} 45% ${5 + light * 29}%)`);
  sky.addColorStop(1, `hsl(215 39% ${11 + light * 49}%)`);
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, width, height);
  const halo = ctx.createRadialGradient(x, y, 0, x, y, distance * 0.8);
  halo.addColorStop(0, `rgb(239 141 87 / ${dusk * 0.74})`);
  halo.addColorStop(0.56, `rgb(155 76 92 / ${dusk * 0.22})`);
  halo.addColorStop(1, 'transparent');
  ctx.fillStyle = halo;
  ctx.fillRect(0, 0, width, height);
}

function anchorTrack(points, timestamp, current) {
  if (timestamp < points[0].time || timestamp > points.at(-1).time) return points;
  const next = points.findIndex(({ time }) => time >= timestamp);
  const anchored = points.slice();
  anchored.splice(next, Number(points[next].time === timestamp), { ...current, time: timestamp });
  return anchored;
}

export function trackRuns(points, timestamp) {
  const past = [];
  const future = [];
  let pastRun;
  let futureRun;
  for (let index = 1; index < points.length; index++) {
    const a = points[index - 1];
    const b = points[index];
    if (a.altitude < 0) {
      pastRun = undefined;
      futureRun = undefined;
      if (b.altitude < 0) continue;
    }
    const delta = ((b.azimuth - a.azimuth + 540) % 360) - 180;
    const horizon = -a.altitude / (b.altitude - a.altitude);
    const from = a.altitude < 0 ? horizon : 0;
    const to = b.altitude < 0 ? horizon : 1;
    const now = Math.max(from, Math.min(to, (timestamp - a.time) / (b.time - a.time)));
    const interpolate = (fraction) =>
      fraction === 0
        ? [a.altitude, a.azimuth]
        : fraction === 1
          ? [b.altitude, b.azimuth]
          : [
              Math.max(0, a.altitude + (b.altitude - a.altitude) * fraction),
              a.azimuth + delta * fraction,
            ];
    // Both strokes use exactly the same point at the change in opacity.
    const seam = interpolate(now);
    if (now > from) {
      if (!pastRun) past.push((pastRun = [interpolate(from)]));
      pastRun.push(seam);
    }
    if (to > now) {
      if (!futureRun) future.push((futureRun = [seam]));
      futureRun.push(interpolate(to));
    }
  }
  return [past, future];
}

function drawTrack(ctx, points, timestamp, point, opacity) {
  const [past, future] = trackRuns(points, timestamp);
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  for (const [runs, alpha] of [
    [past, opacity],
    [future, opacity * 0.57],
  ]) {
    ctx.globalAlpha = alpha;
    ctx.beginPath();
    for (const [start, ...rest] of runs) {
      ctx.moveTo(...point(...start));
      for (const position of rest) ctx.lineTo(...point(...position));
    }
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
}

export function chartGeometry(width, height, { treatment, isChart, projection }) {
  const centerX = width / 2;
  const centerY = height / 2;
  const radius = treatment
    ? Math.hypot(centerX, centerY) * treatment.zoom
    : Math.min(width, height) / 2 - (isChart ? 37 : 18);
  const point = (altitude, azimuth) => {
    const [x, y] = project(altitude, azimuth, projection);
    return [centerX + x * radius, centerY + y * radius];
  };
  return { width, height, centerX, centerY, radius, point };
}

function daylightLevel(sun, treatment) {
  return treatment && !treatment.atmosphere
    ? 0
    : Math.max(0, Math.min(1, (sun.altitude + 18) / 30));
}

const foreground = '#dfdfdf';

function paintSky(
  ctx,
  { width, height, centerX, centerY, radius, point },
  { sun, isChart, treatment },
) {
  const daylight = daylightLevel(sun, treatment);

  ctx.save();
  ctx.clearRect(0, 0, width, height);
  const sky = ctx.createRadialGradient(centerX, centerY, 0, centerX, centerY, radius);
  sky.addColorStop(0, `hsl(0 0% ${(treatment?.centerLight ?? 6) + daylight * 12}%)`);
  sky.addColorStop(1, `hsl(0 0% ${(treatment?.edgeLight ?? 9) + daylight * 18}%)`);
  circle(ctx, centerX, centerY, radius);
  ctx.fillStyle = sky;
  ctx.fill();
  ctx.save();
  ctx.clip();
  if (treatment?.atmosphere) atmosphere(ctx, width, height, sun, point);

  if (isChart) {
    ctx.lineWidth = 0.7;
    ctx.strokeStyle = '#77797555';
    for (const altitude of [30, 60]) {
      const [, y] = point(altitude, 0);
      circle(ctx, centerX, centerY, centerY - y);
      ctx.stroke();
      ctx.fillStyle = '#8c8d85';
      ctx.font = '10px Consolas, monospace';
      ctx.fillText(`${altitude}°`, centerX + 6, y + 12);
    }
    ctx.beginPath();
    for (const azimuth of [0, 45, 90, 135, 180, 225, 270, 315]) {
      ctx.moveTo(centerX, centerY);
      ctx.lineTo(...point(0, azimuth));
    }
    ctx.stroke();
  }

  ctx.restore();
  ctx.restore();
}

export function drawSkyLayer(ctx, width, height, model) {
  paintSky(ctx, chartGeometry(width, height, model), model);
}

export function drawChart(ctx, width, height, model) {
  paintSky(ctx, chartGeometry(width, height, model), model);
  paintOrbits(ctx, width, height, model);
}

export function drawOrbitLayer(ctx, width, height, model) {
  ctx.clearRect(0, 0, width, height);
  paintOrbits(ctx, width, height, model);
}

function paintOrbits(ctx, width, height, model) {
  const {
    bodies,
    sun,
    tracks,
    selected,
    timestamp,
    showPaths,
    isChart,
    treatment,
    satellites = [],
    animateSatellites = false,
    site,
  } = model;
  const geometry = chartGeometry(width, height, model);
  const { centerX, centerY, radius, point } = geometry;
  const daylight = daylightLevel(sun, treatment);
  ctx.save();
  ctx.save();
  circle(ctx, centerX, centerY, radius);
  ctx.clip();

  if (showPaths) {
    for (const { body, points } of tracks) {
      if (body === 'Moon' || body === selected) continue;
      ctx.strokeStyle = '#999b98';
      ctx.lineWidth = 0.72;
      const current = bodies.find((position) => position.body === body);
      drawTrack(
        ctx,
        anchorTrack(points, timestamp, current),
        timestamp,
        point,
        treatment?.paths ?? 0.55,
      );
    }
    const moonPath = lunarPass(timestamp, site);
    if (moonPath) {
      const current = bodies.find(({ body }) => body === 'Moon');
      ctx.strokeStyle = selected === 'Moon' ? foreground : '#999b98';
      ctx.lineWidth = selected === 'Moon' ? 1.15 : 0.72;
      drawTrack(
        ctx,
        anchorTrack(moonPath, timestamp, current),
        Infinity,
        point,
        treatment?.moonPath ?? 0.65,
      );
    }
    const active = tracks.find(({ body }) => body === selected);
    if (selected !== 'Moon') {
      ctx.strokeStyle = foreground;
      ctx.lineWidth = 1.35;
      const current = bodies.find((position) => position.body === selected);
      drawTrack(ctx, anchorTrack(active.points, timestamp, current), timestamp, point, 0.65);
    }
  }

  if (!treatment && sun.altitude >= 0) {
    const [x, y] = point(sun.altitude, sun.azimuth);
    circle(ctx, x, y, 5);
    ctx.fillStyle = '#f6f6f6';
    ctx.fill();
  }

  for (const satellite of satellites) {
    ctx.strokeStyle = '#c5c9c6';
    ctx.lineWidth = 0.65;
    ctx.setLineDash([1.3, 3]);
    drawTrack(ctx, satellite.points, timestamp, point, satellite.sunlit ? 0.65 : 0.24);
    ctx.setLineDash([]);
    if (animateSatellites) continue;
    const [x, y] = point(satellite.altitude, satellite.azimuth);
    circle(ctx, x, y, 1.65);
    if (satellite.sunlit) {
      ctx.fillStyle = '#fafafa';
      ctx.fill();
    } else {
      ctx.strokeStyle = '#888b8a';
      ctx.stroke();
    }
    if (isChart) {
      ctx.fillStyle = '#d0d1c9';
      ctx.font = '10px "Segoe UI", sans-serif';
      ctx.fillText(satellite.label, x + 6, y + 3);
    }
  }

  const labels = [];
  for (const body of bodies) {
    if (body.altitude < 0 || (!isChart && daylight > 0.65 && body.body !== 'Moon')) continue;
    const [x, y] = point(body.altitude, body.azimuth);
    if (body.body === 'Moon') {
      moonMarker(ctx, body, sun, point, x, y, 7);
    } else {
      circle(ctx, x, y, body.mag > 5 ? 3 : 3.5);
      ctx.fillStyle = body.mag > 5 ? '#292929' : foreground;
      ctx.fill();
      ctx.strokeStyle = foreground;
      ctx.lineWidth = 0.8;
      ctx.stroke();
    }
    if (isChart) labels.push({ x, y, body: body.body });
  }
  ctx.restore();

  if (treatment) {
    ctx.restore();
    return;
  }
  circle(ctx, centerX, centerY, radius);
  ctx.strokeStyle = isChart ? '#70736a' : '#77797555';
  ctx.lineWidth = 1;
  ctx.stroke();
  if (!isChart) {
    ctx.restore();
    return;
  }

  for (let bearing = 0; bearing < 360; bearing += 5) {
    const angle = bearing * degrees;
    const [x, y] = point(0, bearing);
    const length = bearing % 30 === 0 ? 7 : 3;
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.lineTo(x - length * Math.sin(angle), y - length * Math.cos(angle));
    ctx.strokeStyle = bearing % 30 === 0 ? '#999d90' : '#55594f';
    ctx.stroke();
  }

  ctx.fillStyle = '#e6e5d8';
  ctx.font = '12px Consolas, monospace';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (const [bearing, label] of [
    [0, 'N'],
    [90, 'E'],
    [180, 'S'],
    [270, 'W'],
  ]) {
    const angle = bearing * degrees;
    ctx.fillText(
      label,
      centerX - (radius + 20) * Math.sin(angle),
      centerY - (radius + 20) * Math.cos(angle),
    );
  }

  ctx.textAlign = 'left';
  ctx.textBaseline = 'alphabetic';
  ctx.font = '12px "Segoe UI", sans-serif';
  const used = [];
  for (const { x, y, body } of labels) {
    const textWidth = ctx.measureText(body).width;
    const labelX = x + textWidth + 12 < width ? x + 10 : x - textWidth - 10;
    let labelY = y - 7;
    while (
      used.some(
        ({ left, right, top }) =>
          labelX < right && labelX + textWidth > left && Math.abs(labelY - top) < 14,
      )
    ) {
      labelY += 15;
    }
    used.push({ left: labelX, right: labelX + textWidth, top: labelY });
    ctx.lineWidth = 3;
    ctx.strokeStyle = '#131413';
    ctx.strokeText(body, labelX, labelY);
    ctx.fillStyle = body === selected ? '#faf8ef' : '#bbbdb2';
    ctx.fillText(body, labelX, labelY);
  }
  ctx.restore();
}

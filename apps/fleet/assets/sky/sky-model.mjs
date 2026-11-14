// Astronomy Engine 2.1.19, vendored from the npm package's esm/astronomy.js.
import {
  Body,
  Equator,
  Horizon,
  Illumination,
  Observer,
  Rotation_EQJ_HOR,
} from './astronomy-engine.mjs';

export const observer = new Observer(-41.2846, 174.7791, 0);
export function observerAt(input) {
  const parts = input?.split(',').map((part) => part.trim());
  if (parts?.length !== 2 || parts.some((part) => !part)) return observer;
  const [latitude, longitude] = parts.map(Number);
  return Math.abs(latitude) <= 90 && Math.abs(longitude) <= 180
    ? new Observer(latitude, longitude, 0)
    : observer;
}
export const timeZone = 'Pacific/Auckland';
export const minute = 60_000;
export const skyUpdateInterval = 30_000;
export const orbitUpdateInterval = 1000;
export const projections = {
  stereographic: (distance) => Math.tan(distance / 2),
  equidistant: (distance) => distance / (Math.PI / 2),
  'equal-area': (distance) => Math.SQRT2 * Math.sin(distance / 2),
  orthographic: (distance) => Math.sin(distance),
};

export function skySnapshotTime(timestamp) {
  return Math.floor(timestamp / skyUpdateInterval) * skyUpdateInterval;
}
export const degrees = Math.PI / 180;
export const objects = [
  Body.Moon,
  Body.Mercury,
  Body.Venus,
  Body.Mars,
  Body.Jupiter,
  Body.Saturn,
  Body.Uranus,
  Body.Neptune,
];

const zonedClock = new Intl.DateTimeFormat('en-NZ', {
  timeZone,
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

export function localParts(instant) {
  return Object.fromEntries(
    zonedClock.formatToParts(instant).map(({ type, value }) => [type, value]),
  );
}

export function localDate(instant) {
  const { year, month, day } = localParts(instant);
  return `${year}-${month}-${day}`;
}

export function changeDate(day, offset) {
  const date = new Date(day + 'T12:00:00Z');
  date.setUTCDate(date.getUTCDate() + offset);
  return date.toISOString().slice(0, 10);
}

export function localInstant(day, hour) {
  return parseWellingtonTime(day + `T${String(hour).padStart(2, '0')}:00:00`);
}

export function parseWellingtonTime(input) {
  const wallClock = Date.parse(input + 'Z');
  if (!Number.isFinite(wallClock)) return NaN;
  const mappedClock = (instant) => {
    const { year, month, day, hour, minute } = localParts(instant);
    return (
      Date.parse(`${year}-${month}-${day}T${hour}:${minute}:00Z`) +
      (((instant % 60_000) + 60_000) % 60_000)
    );
  };
  // Test both offsets near a clock change and resolve repeated wall times to the earlier instant.
  const candidates = [-24, 24].map((hour) => {
    const sample = wallClock + hour * 60 * minute;
    return wallClock - (mappedClock(sample) - sample);
  });
  const matches = candidates.filter((instant) => mappedClock(instant) === wallClock);
  return matches.length ? Math.min(...matches) : NaN;
}

function horizontal(body, date, site) {
  const { ra, dec } = Equator(body, date, site, true, true);
  const { altitude, azimuth } = Horizon(date, site, ra, dec);
  return { altitude, azimuth };
}

export function position(body, timestamp, site = observer) {
  return horizontal(body, new Date(timestamp), site);
}

function moonPhase(date, site) {
  // Use vectors from the observer so lunar parallax affects the illuminated fraction.
  const { vec: moon } = Equator(Body.Moon, date, site, true, true);
  const { vec: sun } = Equator(Body.Sun, date, site, true, true);
  const toSun = [sun.x - moon.x, sun.y - moon.y, sun.z - moon.z];
  const cosine =
    -(moon.x * toSun[0] + moon.y * toSun[1] + moon.z * toSun[2]) /
    (Math.hypot(moon.x, moon.y, moon.z) * Math.hypot(...toSun));
  return (1 + cosine) / 2;
}

export function bodyPositions(timestamp, site = observer) {
  const date = new Date(timestamp);
  return objects.map((body) => {
    const { mag } = Illumination(body, date);
    return {
      body,
      ...horizontal(body, date, site),
      mag,
      phase_fraction: body === Body.Moon ? moonPhase(date, site) : undefined,
    };
  });
}

export function tracksForNight(start, end, site = observer) {
  return objects.map((body) => {
    const points = [];
    for (let time = start; time <= end; time += 5 * minute) {
      points.push({ time, ...position(body, time, site) });
    }
    return { body, points };
  });
}

const lunarTracks = new Map();

export function lunarPass(timestamp, site = observer) {
  const day = Math.floor(timestamp / 86_400_000);
  const key = `${day}/${site.latitude}/${site.longitude}`;
  if (!lunarTracks.has(key)) {
    const start = (day - 1) * 86_400_000;
    const end = (day + 2) * 86_400_000;
    const visibleRuns = [];
    let previous = { time: start, ...position(Body.Moon, start, site) };
    let points = previous.altitude >= 0 ? [previous] : undefined;
    for (let time = start + 5 * minute; time <= end; time += 5 * minute) {
      const current = { time, ...position(Body.Moon, time, site) };
      if (current.altitude >= 0) {
        points ??= [previous];
        points.push(current);
      } else if (points) {
        points.push(current);
        visibleRuns.push(points);
        points = undefined;
      }
      previous = current;
    }
    if (points) visibleRuns.push(points);
    if (lunarTracks.size >= 12) lunarTracks.clear();
    lunarTracks.set(key, visibleRuns);
  }
  const distance = (points) =>
    Math.max(points[0].time - timestamp, timestamp - points.at(-1).time, 0);
  return lunarTracks
    .get(key)
    .reduce(
      (nearest, points) => (!nearest || distance(points) < distance(nearest) ? points : nearest),
      null,
    );
}

export function prepareStars(stars) {
  return stars.map(([hours, dec, magnitude, name]) => {
    const ra = hours * 15 * degrees;
    const lat = dec * degrees;
    return {
      x: Math.cos(lat) * Math.cos(ra),
      y: Math.cos(lat) * Math.sin(ra),
      z: Math.sin(lat),
      magnitude,
      name,
    };
  });
}

export function starsAboveHorizon(stars, timestamp, limit, site = observer) {
  const { rot } = Rotation_EQJ_HOR(new Date(timestamp), site);
  const visible = [];
  // The library's horizontal frame is north, west, up. Row vectors match RotateVector.
  for (const { x, y, z, magnitude, name } of stars) {
    if (magnitude > limit) continue;
    const up = rot[0][2] * x + rot[1][2] * y + rot[2][2] * z;
    if (up < 0) continue;
    const north = rot[0][0] * x + rot[1][0] * y + rot[2][0] * z;
    const west = rot[0][1] * x + rot[1][1] * y + rot[2][1] * z;
    visible.push({
      altitude: Math.atan2(up, Math.hypot(north, west)) / degrees,
      azimuth: (Math.atan2(-west, north) / degrees + 360) % 360,
      magnitude,
      name,
    });
  }
  return visible;
}

export function project(altitude, azimuth, projection) {
  const zenithDistance = (90 - altitude) * degrees;
  const radiusFor = Object.hasOwn(projections, projection)
    ? projections[projection]
    : projections.equidistant;
  const r = radiusFor(zenithDistance);
  // Looking up with north at top places east on the left.
  return [-r * Math.sin(azimuth * degrees), -r * Math.cos(azimuth * degrees)];
}

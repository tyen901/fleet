import {
  ecfToLookAngles,
  eciToEcf,
  gstime,
  json2satrec,
  propagate,
  sunPos,
  jday,
} from './satellite-engine.mjs';
import { degrees, minute, observer } from './sky-model.mjs';

const maxAge = 5 * 24 * 60 * minute;
const earthRadius = 6378.137;

export function prepareSatellites(records) {
  return records.map(({ label, ...elements }) => ({
    label,
    epoch: Date.parse(elements.EPOCH + 'Z'),
    period: 1440 / elements.MEAN_MOTION,
    orbit: json2satrec(elements),
  }));
}

export function satellitePosition(satellite, timestamp, sun = true, site = observer) {
  if (Math.abs(timestamp - satellite.epoch) > maxAge) return null;
  const date = new Date(timestamp);
  const state = propagate(satellite.orbit, date);
  if (!state?.position) return null;
  const { elevation, azimuth } = ecfToLookAngles(
    {
      latitude: site.latitude * degrees,
      longitude: site.longitude * degrees,
      height: site.height / 1000,
    },
    eciToEcf(state.position, gstime(date)),
  );
  const speed = Math.hypot(state.velocity.x, state.velocity.y, state.velocity.z);
  let sunlit = true;
  if (sun) {
    const {
      rsun: [x, y, z],
    } = sunPos(jday(date));
    const distance = Math.hypot(x, y, z);
    const { x: px, y: py, z: pz } = state.position;
    const alongSun = (px * x + py * y + pz * z) / distance;
    // A cylindrical Earth shadow is sufficient for this visibility cue.
    sunlit = alongSun >= 0 || px * px + py * py + pz * pz - alongSun ** 2 > earthRadius ** 2;
  }
  return { altitude: elevation / degrees, azimuth: azimuth / degrees, speed, sunlit };
}

export function satellitesAt(satellites, timestamp, site = observer) {
  return satellites.flatMap((satellite) => {
    const current = satellitePosition(satellite, timestamp, true, site);
    if (!current || current.altitude < 0) return [];
    const points = [];
    for (let offset = -2 * minute; offset <= 2 * minute; offset += 10_000) {
      const time = timestamp + offset;
      const position = offset ? satellitePosition(satellite, time, false, site) : current;
      if (position) points.push({ time, ...position });
    }
    return [{ ...current, label: satellite.label, points }];
  });
}

export function satelliteMotion(satellites, timestamp, interval, site) {
  return satellites.flatMap((satellite) => {
    const from = satellitePosition(satellite, timestamp, true, site);
    const to = satellitePosition(satellite, timestamp + interval, true, site);
    if (!from || !to || (from.altitude < 0 && to.altitude < 0)) return [];
    return [{ label: satellite.label, from, to }];
  });
}

export async function loadSatellites() {
  const response = await fetch(new URL('./satellites.json', import.meta.url));
  if (!response.ok) throw new Error(`Satellite catalogue returned HTTP ${response.status}`);
  return prepareSatellites((await response.json()).objects);
}

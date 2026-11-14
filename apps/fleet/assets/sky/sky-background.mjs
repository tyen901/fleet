import { chartGeometry, drawOrbitLayer, drawSkyLayer } from './sky-chart.mjs';
import { loadSatellites, satelliteMotion, satellitesAt } from './satellite-tracks.mjs';
import {
  bodyPositions,
  observerAt,
  orbitUpdateInterval,
  position,
  prepareStars,
  skySnapshotTime,
  starsAboveHorizon,
  tracksForNight,
} from './sky-model.mjs';

const treatment = {
  zoom: 1,
  stars: 0.72,
  paths: 0.32,
  moonPath: 0.56,
  atmosphere: true,
};

let catalogue;
let satellites;
const nightTracks = new Map();

function loadCatalogue() {
  catalogue ??= (async () => {
    const response = await fetch(new URL('./stars.json', import.meta.url));
    if (!response.ok) throw new Error(`Wellington catalogue returned HTTP ${response.status}`);
    return prepareStars((await response.json()).stars);
  })();
  return catalogue;
}

function skyAt(stars, timestamp, site) {
  const day = Math.floor(timestamp / 86_400_000) * 86_400_000;
  const key = `${day}/${site.latitude}/${site.longitude}`;
  if (!nightTracks.has(key)) {
    if (nightTracks.size >= 10) nightTracks.clear();
    nightTracks.set(key, tracksForNight(day, day + 86_400_000, site));
  }
  return {
    timestamp,
    selected: 'Moon',
    isChart: false,
    showPaths: true,
    site,
    stars: starsAboveHorizon(stars, timestamp, 6.5, site),
    sun: position('Sun', timestamp, site),
    tracks: nightTracks.get(key),
  };
}

class WellingtonSky extends HTMLElement {
  static observedAttributes = ['instant', 'location'];

  #canvas;
  #context;
  #skyLayer;
  #skyContext;
  #markers = new Map();
  #motions = [];
  #orbitTime;
  #timestamp;
  #projected = [];
  #paintedSky;
  #stars;
  #satellites;
  #site = observerAt(null);
  #starTime;
  #model;
  #frame;
  #resize = new ResizeObserver(() => this.#drawLayers());
  #live = () => {
    this.renderAt(Date.now());
    this.#scheduleLive();
  };

  #scheduleLive() {
    if (this.#motions.length) {
      this.#frame = requestAnimationFrame(this.#live);
    } else {
      this.#timer = setTimeout(
        this.#live,
        orbitUpdateInterval - (Date.now() % orbitUpdateInterval),
      );
    }
  }

  #timer;

  async connectedCallback() {
    if (!this.#canvas) {
      this.#canvas = document.createElement('canvas');
      this.#context = this.#canvas.getContext('2d');
      this.#skyLayer = document.createElement('canvas');
      this.#skyContext = this.#skyLayer.getContext('2d', { alpha: false });
      this.append(this.#skyLayer, this.#canvas);
    }
    this.#resize.observe(this);
    try {
      this.#stars = await loadCatalogue();
      satellites ??= loadSatellites().catch((error) => {
        console.error(error);
        return [];
      });
      this.#satellites = await satellites;
      if (this.isConnected) this.#update();
    } catch (error) {
      console.error(error);
    }
  }

  disconnectedCallback() {
    cancelAnimationFrame(this.#frame);
    clearTimeout(this.#timer);
    this.#resize.disconnect();
  }

  attributeChangedCallback(name) {
    if (name === 'location') {
      this.#site = observerAt(this.getAttribute('location'));
      this.#starTime = undefined;
      this.#orbitTime = undefined;
    }
    this.#update();
  }

  #update() {
    if (!this.isConnected || !this.#satellites) return;
    cancelAnimationFrame(this.#frame);
    clearTimeout(this.#timer);
    const fixed = Date.parse(this.getAttribute('instant'));
    if (Number.isFinite(fixed)) this.renderAt(fixed);
    else this.#live();
  }

  renderAt(timestamp) {
    if (!this.isConnected || !this.#satellites) return;
    this.#timestamp = timestamp;
    const starTime = skySnapshotTime(timestamp);
    const orbitTime = Math.floor(timestamp / orbitUpdateInterval) * orbitUpdateInterval;
    let redraw = false;
    if (this.#starTime !== starTime) {
      this.#model = skyAt(this.#stars, starTime, this.#site);
      this.#starTime = starTime;
      this.#paintedSky = undefined;
      redraw = true;
    }
    if (this.#orbitTime !== orbitTime || redraw) {
      this.#orbitTime = orbitTime;
      this.#model.timestamp = orbitTime;
      this.#model.bodies = bodyPositions(orbitTime, this.#site);
      this.#model.satellites = satellitesAt(this.#satellites, orbitTime, this.#site);
      this.#motions = satelliteMotion(this.#satellites, orbitTime, orbitUpdateInterval, this.#site);
      this.#syncMarkers();
      this.#drawLayers();
      return;
    }
    this.#drawMarkers();
  }

  #syncMarkers() {
    for (const [name, marker] of this.#markers) {
      if (!this.#motions.some(({ label }) => label === name)) {
        marker.remove();
        this.#markers.delete(name);
      }
    }
    for (const { label } of this.#motions) {
      if (this.#markers.has(label)) continue;
      const marker = document.createElement('span');
      marker.className = 'sky-satellite';
      marker.dataset.satellite = label;
      this.#markers.set(label, marker);
      this.append(marker);
    }
  }

  #drawMarkers() {
    const fraction = (this.#timestamp - this.#orbitTime) / orbitUpdateInterval;
    for (const { marker, from, to, x, y, dx, dy } of this.#projected) {
      const altitude = from.altitude + (to.altitude - from.altitude) * fraction;
      const hidden = altitude < 0;
      if (marker.hidden !== hidden) marker.hidden = hidden;
      if (!hidden)
        marker.style.transform = `translate3d(${x + dx * fraction}px,${y + dy * fraction}px,0)`;
    }
  }

  #drawLayers() {
    const { clientWidth: width, clientHeight: height } = this;
    if (!this.#model || !this.#context || !this.#skyContext || !width || !height) return;
    const scale = window.devicePixelRatio || 1;
    const pixelWidth = Math.round(width * scale);
    const pixelHeight = Math.round(height * scale);
    if (pixelWidth !== this.#canvas.width || pixelHeight !== this.#canvas.height) {
      this.#canvas.width = pixelWidth;
      this.#canvas.height = pixelHeight;
    }
    this.#context.setTransform(pixelWidth / width, 0, 0, pixelHeight / height, 0, 0);
    const model = {
      ...this.#model,
      projection: 'stereographic',
      treatment,
      animateSatellites: true,
    };
    const { point } = chartGeometry(width, height, model);
    this.#projected = this.#motions.map(({ label, from, to }) => {
      const marker = this.#markers.get(label);
      const [x, y] = point(Math.max(-1, from.altitude), from.azimuth);
      const [endX, endY] = point(Math.max(-1, to.altitude), to.azimuth);
      marker.classList.toggle('sky-satellite--shadow', !from.sunlit);
      return { marker, from, to, x, y, dx: endX - x, dy: endY - y };
    });
    const layerKey = `${width},${height},${scale}`;
    if (this.#paintedSky !== layerKey) {
      this.#skyLayer.width = pixelWidth;
      this.#skyLayer.height = pixelHeight;
      this.#skyContext.setTransform(pixelWidth / width, 0, 0, pixelHeight / height, 0, 0);
      drawSkyLayer(this.#skyContext, width, height, model);
      this.#paintedSky = layerKey;
    }
    drawOrbitLayer(this.#context, width, height, model);
    this.#drawMarkers();
    this.dataset.ready = 'true';
  }
}

customElements.define('wellington-sky', WellingtonSky);

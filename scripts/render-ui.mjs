import { spawn } from 'node:child_process';
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { createServer as createHttpServer } from 'node:http';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const testRoot = path.join(repoRoot, 'target', 'ui-render');
const configRoot = path.join(testRoot, 'config');
const captureRoot = path.join(testRoot, 'captures');
const profileRoot = path.join(testRoot, 'profile');
const secondProfileRoot = path.join(testRoot, 'second-profile');
const cdpPort = 9333;
const cdpHost = '127.0.0.1';
const previewWidth = 420;
const previewHeight = 560;
const exampleGameDirectory = String.raw`D:\Steam\steamapps\common\Arma 3`;
const exampleModpackFolder = String.raw`D:\Mods\Saturn Unit`;
const executable = path.join(
  repoRoot,
  'target',
  'debug',
  process.platform === 'win32' ? 'fleet.exe' : 'fleet',
);

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

function ensureCdpPortAvailable() {
  const server = createServer();
  return new Promise((resolve, reject) => {
    server.once('error', (error) => {
      if (error?.code === 'EADDRINUSE') {
        reject(
          new Error(
            `Fleet UI renderer cannot use CDP port ${cdpPort}: it is already in use. Close the process using that port and rerun.`,
          ),
        );
      } else {
        reject(error);
      }
    });
    server.listen({ host: cdpHost, port: cdpPort, exclusive: true }, () => {
      server.close((error) => (error ? reject(error) : resolve()));
    });
  });
}

class CdpClient {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      if (!message.id) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result);
    };
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.onopen = resolve;
      socket.onerror = () => reject(new Error(`Could not connect to ${url}`));
    });
    return new CdpClient(socket);
  }

  call(method, params = {}) {
    const id = this.nextId++;
    const result = new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
    this.socket.send(JSON.stringify({ id, method, params }));
    return result;
  }

  async evaluate(expression) {
    const response = await this.call('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (response.exceptionDetails) {
      throw new Error(response.exceptionDetails.text ?? 'Browser evaluation failed');
    }
    return response.result.value;
  }

  async waitFor(expression, description, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await this.evaluate(expression)) return;
      await delay(100);
    }
    throw new Error(`Timed out waiting for ${description}`);
  }

  async clickText(label, scope = 'body') {
    const clicked = await this.evaluate(`(() => {
      const button = [...document.querySelectorAll(${JSON.stringify(scope + ' button')})]
        .find((candidate) => candidate.textContent.trim() === ${JSON.stringify(label)} && !candidate.disabled && candidate.getClientRects().length);
      if (!button) return false;
      button.click();
      return true;
    })()`);
    if (!clicked) throw new Error(`Enabled button not found: ${label}`);
    await delay(150);
  }

  async click(selector) {
    const clicked = await this.evaluate(`(() => {
      const element = document.querySelector(${JSON.stringify(selector)});
      if (!element || element.disabled) return false;
      element.click();
      return true;
    })()`);
    if (!clicked) throw new Error(`Enabled element not found: ${selector}`);
    await delay(150);
  }

  async setInput(selector, value) {
    const changed = await this.evaluate(`(() => {
      const input = document.querySelector(${JSON.stringify(selector)});
      if (!input) return false;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(input, ${JSON.stringify(value)});
      input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' }));
      return true;
    })()`);
    if (!changed) throw new Error(`Input not found: ${selector}`);
    await delay(100);
  }

  async capture(filename) {
    const controls = await this.evaluate(`(() => {
      // The footer's back affordance is a named exception: icon+label in one
      // control, so it carries its own (tighter) padding, not the shared 12px.
      const buttons = [...document.querySelectorAll('.btn')]
        .filter((button) => !button.classList.contains('page-footer__back'));
      const fields = [...document.querySelectorAll('.field__input, .select')];
      return {
        buttonsConsistent: buttons.every((button) => {
          const style = getComputedStyle(button);
          if (!button.getClientRects().length) return true;
          const padding = button.matches('.mod-list__row .btn--icon, .profile-row__main > .btn--icon')
            ? '0px'
            : button.matches('.profile-row__buttons .btn') ? '8px' : '12px';
          return button.getBoundingClientRect().height === 34 &&
            style.paddingLeft === padding &&
            style.paddingRight === padding &&
            style.whiteSpace === 'nowrap';
        }),
        fieldsConsistent: fields.every((field) => field.getBoundingClientRect().height === 34),
        unlabelledIconButtons: [...document.querySelectorAll('.btn--icon')]
          .filter((button) => !button.getAttribute('aria-label')).length,
        iconsOutsideIconButtons: [...document.querySelectorAll('.btn .ico')]
          .filter((icon) => !icon.closest('.btn--icon')).length,
        // There is no top chrome at all; every page is body + bottom action bar.
        hasNoPageHeader: document.querySelectorAll('.page-header').length === 0,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
      };
    })()`);
    if (
      !controls.buttonsConsistent ||
      !controls.fieldsConsistent ||
      controls.unlabelledIconButtons > 0 ||
      controls.iconsOutsideIconButtons > 0 ||
      !controls.hasNoPageHeader ||
      controls.viewportWidth !== previewWidth ||
      controls.viewportHeight !== previewHeight
    ) {
      throw new Error(`Inconsistent controls before capturing ${filename}`);
    }
    const result = await this.call('Page.captureScreenshot', {
      format: 'png',
      fromSurface: true,
      captureBeyondViewport: false,
    });
    const png = Buffer.from(result.data, 'base64');
    const width = png.readUInt32BE(16);
    const height = png.readUInt32BE(20);
    if (width !== previewWidth || height !== previewHeight) {
      throw new Error(
        `Expected a ${previewWidth}x${previewHeight} capture, got ${width}x${height} for ${filename}`,
      );
    }
    await writeFile(path.join(captureRoot, filename), png);
  }

  close() {
    this.socket.close();
  }
}

// Windows keeps a directory handle open while an image viewer or indexer has a
// capture in hand, so the capture directory is emptied in place rather than
// removed. Config and profile state are disposable and still get a clean tree.
async function clearDirectoryContents(directory) {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
  await Promise.all(
    entries.map((entry) => rm(path.join(directory, entry.name), { recursive: true, force: true })),
  );
}

async function seedDummyConfig(slowPort) {
  await Promise.all(
    [configRoot, profileRoot, secondProfileRoot].map((directory) =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
  await mkdir(configRoot, { recursive: true });
  await mkdir(captureRoot, { recursive: true });
  await mkdir(profileRoot, { recursive: true });
  await clearDirectoryContents(captureRoot);

  const settings = {
    arma3_default_args: '-noPause -noSplash -skipIntro -noLauncher',
    arma3_game_dir: '',
    arma3_launch_method: process.platform === 'win32' ? 'arma3-exe' : 'steam',
    arma3_custom_launch_template:
      process.platform === 'win32' ? 'arma3_x64.exe $ARGS $MODS' : 'steam $ARGS $MODS',
    onboarding_completed: false,
    show_profile_icons: false,
    debug_log_to_disk: false,
    auto_check_profiles_on_startup: false,
    auto_check_on_startup: false,
  };
  const profiles = {
    profiles: [
      {
        id: 'ui-test-profile',
        name: 'Saturn Unit',
        source: `http://127.0.0.1:${slowPort}/unavailable/repo.json`,
        destination: profileRoot,
        arma3_server: null,
        swifty_repo_revision: '',
        launch_params: '',
        additional_mod_folders: [String.raw`D:\Mods\@ace`, String.raw`D:\Mods\@cba_a3`],
      },
      {
        id: 'ui-test-second-profile',
        name: 'Triton Unit',
        source: `http://127.0.0.1:${slowPort}/repo.json`,
        destination: secondProfileRoot,
        arma3_server: null,
        swifty_repo_revision: '',
        launch_params: '',
        additional_mod_folders: [],
      },
    ],
  };

  await writeFile(path.join(configRoot, 'settings.json'), JSON.stringify(settings));
  await writeFile(path.join(configRoot, 'profiles.json'), JSON.stringify(profiles));
}

async function waitForTarget(timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://${cdpHost}:${cdpPort}/json/list`);
      const targets = await response.json();
      const target = targets.find(
        (candidate) =>
          candidate.type === 'page' &&
          candidate.title === 'Dioxus app' &&
          candidate.url === 'http://dioxus.index.html/',
      );
      if (target?.webSocketDebuggerUrl) return target.webSocketDebuggerUrl;
    } catch {
      // Fleet or WebView2 is still starting.
    }
    await delay(150);
  }
  throw new Error(
    'Fleet WebView2 debugging target did not become available with the expected Dioxus title and URL',
  );
}

async function runFlow(client, repo) {
  await client.call('Runtime.enable');
  await client.call('Page.enable');
  await client.call('Emulation.setEmulatedMedia', {
    features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }],
  });
  await client.call('Emulation.setDeviceMetricsOverride', {
    width: previewWidth,
    height: previewHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await client.waitFor(`document.querySelector('input[aria-label="Arma 3 Directory"]')`, 'onboarding directory field');
  const onboardingText = await client.evaluate(`document.querySelector('.page-frame').innerText.trim().split(/\\s+/).join(' ')`);
  if (onboardingText !== 'Arma 3 Directory Auto Continue') {
    throw new Error(`Unexpected onboarding text: ${JSON.stringify(onboardingText)}`);
  }

  const viewport = await client.evaluate(
    `({ width: window.innerWidth, height: window.innerHeight, scale: window.devicePixelRatio })`,
  );
  if (
    viewport.width !== previewWidth ||
    viewport.height !== previewHeight ||
    Math.abs(viewport.scale - 1) > 0.001
  ) {
    throw new Error(
      `Expected the fixed ${previewWidth}x${previewHeight} portrait viewport at 1x, got ${viewport.width}x${viewport.height} at ${viewport.scale}x`,
    );
  }

  await client.capture('01-onboarding.png');
  await client.setInput('input[type="text"]', exampleGameDirectory);
  await client.capture('01a-onboarding-ready.png');
  await client.clickText('Continue');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');
  await client.waitFor(
    `(() => {
      const sky = document.querySelector('wellington-sky[data-ready="true"]');
      const bounds = sky?.getBoundingClientRect();
      return bounds?.width === innerWidth && bounds.height === innerHeight
        && getComputedStyle(sky).pointerEvents === 'none'
        && sky.getAttribute('aria-hidden') === 'true';
    })()`,
    'full-window Wellington background behind the profile cards',
  );
  const firstProfile = '.profile-row:first-child';
  const secondProfile = '.profile-row:nth-child(2)';
  const openActions = async (profile = firstProfile) => {
    await client.click(profile + ' [popovertarget]');
    await client.waitFor(`document.querySelector('.profile-actions:popover-open')`, 'open profile actions');
  };
  const sky = "document.querySelector('wellington-sky')";
  const setSkyTime = (instant) =>
    client.evaluate(`${sky}.setAttribute('instant', ${JSON.stringify(instant)})`);
  const skyColor = () => client.evaluate(`(() => {
    const ctx = ${sky}.querySelector('canvas').getContext('2d');
    return [...ctx.getImageData(210, 410, 1, 1).data];
  })()`);
  await setSkyTime('2026-11-14T07:30:00Z');
  await client.capture('02a-profiles-default.png');
  await setSkyTime('2026-11-14T00:00:00Z');
  const dayColor = await skyColor();
  await client.capture('02m-profiles-sky-daylight.png');
  await setSkyTime('2026-11-14T10:30:00Z');
  const nightColor = await skyColor();
  await client.capture('02n-profiles-sky-night.png');
  if (dayColor[2] <= dayColor[0] || dayColor[2] - nightColor[2] < 40) {
    throw new Error('The profile sky should shift from blue daylight to a dark night gradient');
  }
  const starlessNight = await client.evaluate(`(() => {
    const [background, orbits] = ${sky}.querySelectorAll('canvas');
    const pixels = background.getContext('2d').getImageData(0, 0, background.width, background.height);
    // At this instant the Sun is well below the horizon, leaving a vertical gradient.
    for (let y = 1; y < pixels.height - 1; y++) {
      const start = y * pixels.width * 4;
      for (let x = 1; x < pixels.width - 1; x++) {
        for (let channel = 0; channel < 3; channel++) {
          if (Math.abs(pixels.data[start + 4 * x + channel] - pixels.data[start + 4 + channel]) > 2) return false;
        }
      }
    }
    const overlay = orbits.getContext('2d').getImageData(0, 0, orbits.width, orbits.height);
    return overlay.data.some((value, index) => index % 4 === 3 && value > 0);
  })()`);
  if (!starlessNight) throw new Error('Night should show a smooth gradient and a separate orbital overlay');
  await setSkyTime('2026-11-17T08:36:20Z');
  await client.waitFor(
    `${sky}.querySelector('[data-satellite="Tiangong"]:not([hidden])')?.style.transform`,
    'visible Tiangong marker',
  );
  await client.capture('02q-profiles-sky-satellite.png');
  await client.evaluate(`${sky}.removeAttribute('instant')`);
  const checkForUpdates = async (profile, capture) => {
    await openActions(profile);
    if (capture) await client.capture(capture);
    await client.clickText('Check for updates', '.profile-actions:popover-open');
  };
  const validateFromMenu = async (profile) => {
    await openActions(profile);
    await client.clickText('Validate files', '.profile-actions:popover-open');
  };
  const noSyncOnCard = (profile) => `![...document.querySelectorAll(${JSON.stringify(profile + ' .profile-row__buttons button')})]
    .some(button => button.textContent.trim() === 'Sync')`;
  if (!(await client.evaluate(noSyncOnCard('.profile-row')))) {
    throw new Error('Unchecked profiles should not show Sync on their cards');
  }
  const nameSelector = JSON.stringify(firstProfile + ' .profile-row__name');
  if (!(await client.evaluate(`document.querySelector(${nameSelector})?.tagName === 'SPAN'`))) {
    throw new Error('Profile title should be display-only');
  }
  await client.click(firstProfile + ' .profile-row__name');
  if (!(await client.evaluate(`document.querySelector('.profiles-page__list')`))) {
    throw new Error('Profile title should leave the profile cards open');
  }
  await checkForUpdates(firstProfile, '02k-profiles-check-menu.png');
  await client.waitFor(`!document.querySelector('.profile-operation')`, 'finished first update check');
  if (!(await client.evaluate(noSyncOnCard(firstProfile)))) {
    throw new Error('An unavailable source should not show Sync on the card');
  }
  await validateFromMenu(firstProfile);
  await client.waitFor(`(() => {
    const notice = document.querySelector('.profile-row:first-child .note--bad');
    return notice?.querySelector('[role="alert"]')?.textContent.includes('sync source')
      && getComputedStyle(notice).borderTopWidth === '0px'
      && !document.querySelector('.profile-operation');
  })()`, 'inline sync error', 20_000);
  await client.capture('02-profiles-error.png');
  await openActions();
  const profileActions = await client.evaluate(`[...document.querySelectorAll('.profile-actions:popover-open button')].map(button => button.textContent.trim())`);
  if (!profileActions.includes('Validate files') || profileActions.includes('Force sync')) {
    throw new Error('Profile actions should offer Validate files without Force sync');
  }
  await client.capture('02f-profile-actions.png');
  await client.clickText('Edit profile', '.profile-actions:popover-open');
  await client.waitFor(`document.querySelector('.mod-list__row input:not([readonly])')`, 'direct profile editor');
  if (await client.evaluate(`document.body.innerText.includes("Couldn't load the sync source.")`)) {
    throw new Error('Sync failure leaked into the profile editor');
  }
  await client.clickText('Cancel');

  await client.click('[aria-label="Settings"]');
  await client.waitFor(
    `[...document.querySelectorAll('.section__title')]
      .some((title) => title.textContent.trim() === 'Updates')`,
    'settings view',
  );
  await client.capture('03-settings-top.png');
  const settingsTop = await client.evaluate(`(() => {
    const sectionTitles = [...document.querySelectorAll('.section .section__title')]
      .map((title) => title.textContent.trim());
    const gameRow = [...document.querySelectorAll('.field-row')]
      .find((row) => row.querySelector('.field-row__title')?.textContent.trim() === 'Arma 3 Directory');
    const resetSlots = [...document.querySelectorAll('.field-reset')];
    const argsRow = [...document.querySelectorAll('.field-row')]
      .find(row => row.querySelector('.field-row__title')?.textContent.trim() === 'Default launch arguments');
    const argsInput = argsRow.querySelector('input');
    const cancel = [...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Cancel');
    const toggles = [...document.querySelectorAll('.field-row input[type="checkbox"]')];
    const save = [...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Save');
    return {
      sectionTitles,
      gameDirectoryButtons: gameRow.querySelectorAll('button').length,
      hasBrowseIcon: Boolean(gameRow.querySelector('.btn--icon[aria-label="Browse for Arma 3 directory"]'))
        && Boolean(gameRow.querySelector('input[aria-label="Arma 3 Directory"]')),
      defaultArgsFullWidth: Math.abs(argsInput.getBoundingClientRect().width - argsRow.getBoundingClientRect().width) < 1,
      hasAutoAction: [...gameRow.querySelectorAll('button')]
        .some((button) => button.textContent.trim() === 'Auto'),
      resetSlotsConsistent: resetSlots.length > 0 && new Set(
        resetSlots.map((slot) => slot.getBoundingClientRect().width),
      ).size === 1,
      hasReservedResetSlot: resetSlots.some((slot) => slot.classList.contains('field-reset--hidden')),
      hasCancel: Boolean(cancel),
      togglesRightAligned: toggles.length > 0 && toggles.every((toggle) => {
        const control = toggle.closest('.field-row__control--actions');
        return control && Math.abs(control.getBoundingClientRect().right - toggle.getBoundingClientRect().right) < 1;
      }),
      saveInitiallyDisabled: Boolean(save?.disabled),
      saveInFooterActions: Boolean(document.querySelector('.page-footer__actions')?.contains(save)),
    };
  })()`);
  if (
    settingsTop.sectionTitles[0] !== 'Updates' ||
    settingsTop.gameDirectoryButtons !== 2 ||
    !settingsTop.hasBrowseIcon ||
    !settingsTop.defaultArgsFullWidth ||
    !settingsTop.hasAutoAction ||
    !settingsTop.resetSlotsConsistent ||
    !settingsTop.hasReservedResetSlot ||
    !settingsTop.hasCancel ||
    !settingsTop.togglesRightAligned ||
    !settingsTop.saveInitiallyDisabled ||
    !settingsTop.saveInFooterActions
  ) {
    throw new Error('Settings ordering or inline control sizing is incorrect');
  }
  const latitude = 'input[aria-label="Latitude"]';
  const longitude = 'input[aria-label="Longitude"]';
  const saveEnabled = `[...document.querySelectorAll('.page-footer button')]
    .some(button => button.textContent.trim() === 'Save' && !button.disabled)`;
  const coordinateError = `document.querySelector('${latitude}').closest('.section')
    .querySelector('.field__error[role="alert"]')?.textContent.trim()`;
  await client.evaluate(`document.querySelector('${latitude}').scrollIntoView({ block: 'center' })`);
  if (await client.evaluate(coordinateError)) throw new Error('Blank sky coordinates show an error');
  await client.capture('04e-settings-sky-default.png');
  await client.setInput(latitude, '-90');
  await client.waitFor(`${coordinateError} === 'Enter latitude from −90 to 90 and longitude from −180 to 180.'`, 'inline sky coordinate validation');
  if (await client.evaluate(saveEnabled)) throw new Error('A single sky coordinate can be saved');
  await client.capture('04g-settings-sky-invalid.png');
  await client.setInput(longitude, '181');
  if (!(await client.evaluate(coordinateError)) || await client.evaluate(saveEnabled)) {
    throw new Error('Out-of-range sky coordinates should show an error and prevent saving');
  }
  await client.setInput(longitude, '180');
  await client.waitFor(`(${saveEnabled}) && !(${coordinateError})`, 'valid sky coordinates clear the error');
  await client.capture('04f-settings-sky-override.png');
  await client.clickText('Save', '.page-footer');
  await client.waitFor(`document.querySelector('wellington-sky[data-ready="true"][location="-90,180"]')`, 'saved sky location');
  await client.capture('02l-profiles-south-pole.png');
  await client.click('[aria-label="Settings"]');
  await client.waitFor(`document.querySelector('${latitude}')?.value === '-90' && document.querySelector('${longitude}')?.value === '180'`, 'persisted sky coordinates');
  await client.setInput(latitude, '');
  await client.setInput(longitude, '');
  await client.clickText('Save', '.page-footer');
  await client.waitFor(`document.querySelector('wellington-sky[data-ready="true"][location="-41.2846,174.7791"]')`, 'restored Wellington sky');
  await client.click('[aria-label="Settings"]');
  await client.waitFor(`document.querySelector('${latitude}')`, 'returned to settings');
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 100000`);
  await delay(100);
  await client.capture('04-settings-bottom.png');
  const modal = '.confirm-dialog:modal';
  // Fluent wraps interpolated values with Unicode bidi isolation marks.
  const withoutIsolation = (value) => value.replace(/[\u2068\u2069]/g, '');
  const waitForConfirmDialog = async (title) => {
    await client.waitFor(`(() => {
      const dialog = document.querySelector(${JSON.stringify(modal)});
      if (!dialog || (${withoutIsolation.toString()})(dialog.querySelector('h2').textContent.trim()) !== ${JSON.stringify(title)}) return false;
      const bounds = dialog.getBoundingClientRect();
      return Math.abs((bounds.left + bounds.right) / 2 - innerWidth / 2) < 1
        && Math.abs((bounds.top + bounds.bottom) / 2 - innerHeight / 2) < 1
        && document.activeElement?.textContent.trim() === 'Cancel'
        && dialog.contains(document.activeElement);
    })()`, 'centered modal with focus on Cancel');
  };
  await client.evaluate(`[...document.querySelectorAll('.field-row')]
    .find(row => row.querySelector('.field-row__title')?.textContent.trim() === 'Reset all settings')
    .querySelector('button').click()`);
  await waitForConfirmDialog('Reset all settings?');
  await client.capture('04b-settings-reset.png');
  await client.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await client.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await client.waitFor(`!document.querySelector('.confirm-dialog:modal')`, 'Escape dismisses modal');
  if (await client.evaluate(`![...document.querySelectorAll('.page-footer button')].find(button => button.textContent.trim() === 'Save')?.disabled`)) {
    throw new Error('Dismissing reset changed the settings draft');
  }
  await client.evaluate(`[...document.querySelectorAll('.field-row')]
    .find(row => row.querySelector('.field-row__title')?.textContent.trim() === 'Reset all settings')
    .querySelector('button').click()`);
  await waitForConfirmDialog('Reset all settings?');
  await client.clickText('Reset', modal);
  await client.waitFor(`!document.querySelector('.confirm-dialog:modal') && [...document.querySelectorAll('.page-footer button')].some(button => button.textContent.trim() === 'Save' && !button.disabled)`, 'settings reset applied to the draft');
  await client.clickText('Cancel', '.page-footer');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view after discarding reset');
  await client.click('[aria-label="Settings"]');
  await client.waitFor(`document.querySelector('.section__title')?.textContent.trim() === 'Updates'`, 'settings restored after discard');

  const openFactoryReset = async () => {
    await client.evaluate(`(() => {
      const row = [...document.querySelectorAll('.field-row')]
        .find(row => row.querySelector('.field-row__title')?.textContent.trim() === 'Factory reset');
      row.scrollIntoView({ block: 'end' });
      row.querySelector('button').click();
    })()`);
    await waitForConfirmDialog('Factory reset?');
  };
  await openFactoryReset();
  await client.capture('04c-settings-factory-reset.png');
  await client.clickText('Cancel', modal);
  await client.waitFor(`!document.querySelector('.confirm-dialog:modal')`, 'factory reset canceled');
  await client.evaluate(`document.querySelector('.select').scrollIntoView({block:'center'})`);
  await client.capture('04a-settings-middle.png');
  await client.evaluate(`(() => {
    const field = document.querySelector('.select');
    field.value = 'custom';
    field.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
  await client.waitFor(`document.querySelector('textarea')`, 'custom launch preview');
  await client.evaluate(`document.querySelector('textarea').scrollIntoView({block:'center'})`);
  await client.capture('04d-settings-custom-launch.png');
  await client.evaluate(`(() => {
    const field = document.querySelector('.select');
    field.value = 'arma3-exe';
    field.dispatchEvent(new Event('change', { bubbles: true }));
    document.querySelector('.page-frame__body').scrollTop = 100000;
  })()`);
  const settingsLayout = await client.evaluate(`(() => {
    const rows = [...document.querySelectorAll('.field-row')];
    const advanced = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'Advanced');
    const actions = [...advanced.querySelectorAll('.field-row__control--actions')];
    return {
      rowLayouts: rows.map((row) => ({
        columns: getComputedStyle(row).gridTemplateColumns.split(' ').length,
        trailing: Boolean(row.querySelector('.field-row__control--actions')),
      })),
      advancedActionsRightAligned: actions.length === 4 && actions.every((control) => {
        const button = control.querySelector('button');
        return Math.abs(
          button.getBoundingClientRect().right - control.getBoundingClientRect().right,
        ) < 1;
      }),
    };
  })()`);
  const rowLayoutsCorrect = settingsLayout.rowLayouts.every(
    (row) => row.columns === (row.trailing ? 2 : 1),
  );
  if (!rowLayoutsCorrect || !settingsLayout.advancedActionsRightAligned) {
    throw new Error('Settings rows did not stack, or Advanced actions were not right aligned');
  }
  await client.evaluate(`(() => {
    const general = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'General');
    general.querySelector('input[type="checkbox"]').click();
  })()`);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Save')?.disabled === false`,
    'enabled settings save action',
  );
  await client.clickText('Cancel');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');

  await client.click('[aria-label="Settings"]');
  await client.waitFor(
    `[...document.querySelectorAll('.section__title')]
      .some((title) => title.textContent.trim() === 'Updates')`,
    'settings view',
  );
  const cancelledValue = await client.evaluate(`(() => {
    const general = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'General');
    return general.querySelector('input[type="checkbox"]').checked;
  })()`);
  if (cancelledValue) throw new Error('Cancel persisted a rejected settings change');
  await client.evaluate(`(() => {
    const general = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'General');
    general.querySelector('input[type="checkbox"]').click();
  })()`);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Save')?.disabled === false`,
    'enabled settings save action',
  );
  await client.clickText('Save');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');
  await client.click('[aria-label="Settings"]');
  await client.waitFor(
    `[...document.querySelectorAll('.section__title')]
      .some((title) => title.textContent.trim() === 'Updates')`,
    'settings view',
  );
  const savedValue = await client.evaluate(`(() => {
    const general = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'General');
    return general.querySelector('input[type="checkbox"]').checked;
  })()`);
  if (!savedValue) throw new Error('Save did not persist the settings draft');
  await client.evaluate(`(() => {
    const general = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'General');
    general.querySelector('input[type="checkbox"]').click();
  })()`);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Save')?.disabled === false`,
    'enabled settings save action',
  );
  await client.clickText('Save');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');

  await client.click('[aria-label="New profile"]');
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .some((button) => button.textContent.trim() === 'Create')`,
    'new profile view',
  );
  const installFolder = path.join(testRoot, 'modpacks', 'Neptune Unit');
  const folderInput = 'input[aria-label="Mod install folder"]';
  const createFields = await client.evaluate(`[...document.querySelectorAll('.form-field__label')].map(label => label.textContent.trim())`);
  if (createFields.join(',') !== 'Name,Sync source URL,Mod install folder' || await client.evaluate(`document.querySelector('.mod-list, .check') !== null`)) {
    throw new Error('New profiles should ask for a name, sync source, and mod install folder');
  }
  const hasFolderPicker = await client.evaluate(`(() => {
    const input = document.querySelector(${JSON.stringify(folderInput)});
    return input?.value === '' && input.placeholder.includes('Mods')
      && input.closest('.field').querySelector('.btn--icon[aria-label="Browse for mod install folder"]');
  })()`);
  if (!hasFolderPicker) throw new Error('Mod install folder needs a placeholder and icon picker');
  await client.capture('05-new-profile.png');
  await client.setInput('.form-field:nth-child(1) .field__input', 'Neptune !');
  await client.setInput('.form-field:nth-child(2) .field__input', 'example.test');
  await client.capture('05a-new-profile-invalid.png');
  await client.setInput('input[aria-label="Sync source URL"]', repo.invalidManifestSource);
  await client.clickText('Test URL');
  await client.waitFor(`[...document.querySelectorAll('[role="status"]')].some(element => element.textContent.includes('Could not load a Swifty manifest'))`, 'invalid remote manifest rejected');
  await client.capture('05c-new-profile-source-error.png');
  await client.setInput('input[aria-label="Sync source URL"]', repo.testSource);
  await client.clickText('Test URL');
  await client.waitFor(`[...document.querySelectorAll('[role="status"]')].some(element => element.textContent.includes('Swifty manifest found'))`, 'valid remote manifest');
  await client.capture('05d-new-profile-source-tested.png');
  await client.setInput('.form-field:nth-child(1) .field__input', 'Neptune Unit');
  await client.setInput('.form-field:nth-child(2) .field__input', 'https://example.com/neptune/repo.json');
  if (await client.evaluate(`![...document.querySelectorAll('button')].find(button => button.textContent.trim() === 'Create')?.disabled`)) {
    throw new Error('A mod install folder is required');
  }
  await client.setInput(folderInput, String.raw`D:\Mods\Neptune Unit`);
  await client.waitFor(`[...document.querySelectorAll('button')].some(button => button.textContent.trim() === 'Create' && !button.disabled)`, 'valid new profile form');
  await client.capture('05b-new-profile-ready.png');
  await client.clickText('Cancel');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');

  await openActions();
  await client.clickText('Edit profile', '.profile-actions:popover-open');
  await client.waitFor(`document.querySelector('.mod-list__row input:not([readonly])')`, 'direct profile editing');
  // Display a realistic example in the editor captures without syncing there.
  await client.setInput('input[aria-label="Mod install folder"]', exampleModpackFolder);
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 0`);
  await delay(100);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .some((button) => button.textContent.trim() === 'Cancel')`,
    'edit profile cancel action',
  );
  await client.capture('07-edit-profile-top.png');
  const customLaunchArguments = '-world=empty -showScriptErrors';
  const defaultsInitiallyEnabled = await client.evaluate(
    `document.querySelector('.field-row .check')?.checked`,
  );
  if (!defaultsInitiallyEnabled) throw new Error('Default launch arguments should start enabled');
  await client.click('.field-row .check');
  await client.waitFor(
    `[...document.querySelectorAll('.form-field__label')]
      .some((label) => label.textContent.trim() === 'Launch arguments')`,
    'custom launch arguments field',
  );
  const savedCustomLaunchArguments = await client.evaluate(`(() => {
    const field = [...document.querySelectorAll('.form-field')]
      .find((candidate) => candidate.querySelector('.form-field__label')?.textContent.trim() === 'Launch arguments');
    const input = field?.querySelector('.field__input');
    if (!input) return false;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(input, ${JSON.stringify(customLaunchArguments)});
    input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' }));
    return input.value;
  })()`);
  if (savedCustomLaunchArguments !== customLaunchArguments) {
    throw new Error('Custom launch arguments could not be entered');
  }
  await client.capture('07-edit-profile-custom-args.png');
  await client.click('.field-row .check');
  await client.waitFor(
    `![...document.querySelectorAll('.form-field__label')]
      .some((label) => label.textContent.trim() === 'Launch arguments')`,
    'hidden default launch arguments field',
  );
  await client.click('.field-row .check');
  await client.waitFor(
    `(() => {
      const field = [...document.querySelectorAll('.form-field')]
        .find((candidate) => candidate.querySelector('.form-field__label')?.textContent.trim() === 'Launch arguments');
      return field?.querySelector('.field__input')?.value === ${JSON.stringify(customLaunchArguments)};
    })()`,
    'restored custom launch arguments',
  );
  await client.click('.field-row .check');
  await client.waitFor(
    `![...document.querySelectorAll('.form-field__label')]
      .some((label) => label.textContent.trim() === 'Launch arguments')`,
    'restored default launch arguments visibility',
  );
  await client.evaluate(`(() => {
    document.querySelector('.mod-list').closest('.section').scrollIntoView({ block: 'start' });
  })()`);
  await delay(100);
  await client.capture('08-edit-profile-mods.png');

  const modControls = await client.evaluate(`[...document.querySelectorAll('.mod-list__row')].map(row => {
    const input = row.querySelector('input');
    const buttons = [...row.querySelectorAll('.btn--icon')];
    return Boolean(input?.getAttribute('aria-label')) && !input.readOnly &&
      buttons.length === 2 && buttons.every(button => button.title && button.getAttribute('aria-label'));
  })`);
  if (modControls.length !== 2 || modControls.some(valid => !valid)) {
    throw new Error('Mod rows need editable paths and labeled browse and remove controls');
  }

  await client.clickText('Add mod');
  const newMod = '.mod-list__row:last-child input';
  await client.waitFor(
    `document.querySelectorAll('.mod-list__row').length === 3 && document.activeElement === document.querySelector(${JSON.stringify(newMod)})`,
    'new mod path focused',
  );
  await client.capture('09-edit-profile-mod-added.png');
  const modPath = String.raw`D:\Mods\@rhs`;
  await client.setInput(newMod, modPath);
  await client.capture('09a-edit-profile-mod-path.png');
  await client.setInput('input[aria-label="Mod install folder"]', profileRoot);
  await client.clickText('Save');
  await client.waitFor(`document.querySelector('.profiles-page')`, 'return to Profiles after saving');
  if (await client.evaluate(`document.querySelector('.profiles-page').innerText.match(/@ace|@cba_a3|@rhs/)`)) {
    throw new Error('Additional mods should only appear in the editor');
  }
  await openActions();
  await client.clickText('Edit profile', '.profile-actions:popover-open');
  await client.waitFor(`document.querySelectorAll('.mod-list__row').length === 3 && document.querySelector(${JSON.stringify(newMod)})?.value === ${JSON.stringify(modPath)}`, 'editable saved mod paths');
  while ((await client.evaluate(`document.querySelectorAll('.mod-list__row').length`)) > 0) {
    await client.click('[aria-label^="Remove mod folder"]');
  }
  await client.clickText('Add mod');
  await client.waitFor(`document.querySelectorAll('.mod-list__row').length === 1`, 'first mod row from empty editor');
  await client.click('[aria-label^="Remove mod folder"]');
  await client.clickText('Save');
  await client.waitFor(`document.querySelector('.profiles-page')`, 'profile card after removing all additional mods');
  await validateFromMenu(firstProfile);
  await client.waitFor(`!document.querySelector('.profile-operation')`, 'verification finished');
  await client.capture('10-verification-result.png');

  const finishOperation = async (expression) => {
    const deadline = Date.now() + 20_000;
    while (Date.now() < deadline) {
      repo.failPendingRequests();
      if (await client.evaluate(expression)) return;
      await delay(100);
    }
    throw new Error('Fixture operation did not finish');
  };

  const waitForProgressFrame = async (profile) => {
    await client.waitFor(`(() => {
      const operation = document.querySelector(${JSON.stringify(profile + ' .profile-operation')});
      if (!operation) return false;
      const clip = operation.closest('.operation-reveal__inner').getBoundingClientRect();
      if (clip.height + 1 < operation.getBoundingClientRect().height) return false;
      const fill = operation.querySelector('.progress-bar-indeterminate .progress-bar-fill');
      if (!fill) return true;
      const fillRect = fill.getBoundingClientRect();
      const trackRect = fill.parentElement.getBoundingClientRect();
      return fillRect.left >= trackRect.left && fillRect.right <= trackRect.right;
    })()`, 'visible progress frame', 2_000);
  };

  await checkForUpdates(secondProfile);
  await client.waitFor(`(() => {
    const row = document.querySelector('.profile-row:nth-child(2)');
    const fill = row.querySelector('.profile-operation .progress-bar-indeterminate[aria-label="Checking for updates"] .progress-bar-fill');
    return fill && getComputedStyle(fill).animationName === 'progress-bar-indeterminate'
      && !row.querySelector('.btn__spinner, .profile-row__buttons');
  })()`, 'indeterminate profile check');
  await waitForProgressFrame(secondProfile);
  await client.capture('02d-profiles-checking.png');
  await finishOperation(`document.querySelector('.profile-row:nth-child(2) [popovertarget]')`);
  if (await client.evaluate(`document.querySelector('.profile-row:nth-child(2) .operation-reveal')`)) {
    throw new Error('Progress should be removed when the check completes');
  }
  await client.capture('02o-profiles-check-complete.png');
  await validateFromMenu(secondProfile);
  await client.waitFor(`(() => {
    const row = document.querySelector('.profile-row:nth-child(2)');
    return row.querySelector('.profile-operation') && !row.querySelector('.profile-row__buttons');
  })()`, 'sync in progress with game actions hidden');
  await waitForProgressFrame(secondProfile);
  await client.capture('02e-profiles-syncing.png');
  await finishOperation(`!document.querySelector('.profile-operation')`);

  await validateFromMenu(secondProfile);
  await client.waitFor(`document.querySelector('.profile-row:nth-child(2) .profile-operation')`, 'verification in progress');
  await waitForProgressFrame(secondProfile);
  await client.capture('10a-verification-progress.png');
  await finishOperation(`!document.querySelector('.profile-operation')`);

  await openActions();
  await client.clickText('Delete profile', '.profile-actions:popover-open');
  await waitForConfirmDialog('Delete “Saturn Unit”?');
  await client.capture('12-delete-confirm.png');
  await client.clickText('Cancel', modal);
  await client.waitFor(`!document.querySelector('.confirm-dialog:modal') && document.activeElement?.hasAttribute('popovertarget') && document.activeElement.closest('.profile-row')?.querySelector('.profile-row__name')?.textContent.trim() === 'Saturn Unit' && document.querySelectorAll('.profile-row').length === 2`, 'delete canceled and focus restored');

  repo.serveManifest([]);
  await validateFromMenu(secondProfile);
  await client.waitFor(`document.querySelectorAll('.profile-row:nth-child(2) .operation-work').length === 2`, 'download and patch tracks');
  const syncMetricsVisible = await client.evaluate(`(() => {
    const tracks = [...document.querySelectorAll('.profile-row:nth-child(2) .operation-work')];
    return tracks[0].querySelector('.operation-work__value')?.textContent.includes('B')
      && tracks[1].querySelector('.operation-work__value')?.textContent.includes('%')
      && tracks.every((track, index) => {
        const row = track.querySelector('.operation-work__bar');
        const bar = row.querySelector('.progress-bar').getBoundingClientRect();
        const metric = row.querySelector('.operation-usage');
        const speed = metric?.querySelector('.operation-usage__value');
        const icon = metric?.querySelector('.operation-usage__icon');
        if (!speed?.textContent.includes('/s') || !icon) return false;
        const speedRect = speed.getBoundingClientRect();
        const iconRect = icon.getBoundingClientRect();
        const center = (bar.top + bar.bottom) / 2;
        return metric.getAttribute('aria-label') === (index === 0 ? 'Network speed' : 'Disk usage')
          && bar.right <= speedRect.left && speedRect.right <= iconRect.left
          && Math.abs(center - (speedRect.top + speedRect.bottom) / 2) < 1
          && Math.abs(center - (iconRect.top + iconRect.bottom) / 2) < 1;
      });
  })()`);
  if (!syncMetricsVisible) throw new Error('Download and patch progress must show values and speeds');
  await waitForProgressFrame(secondProfile);
  await client.capture('02j-profiles-download-patch.png');
  repo.releaseDownload();
  await client.waitFor(`!document.querySelector('.profile-operation')`, 'fixture download complete', 20_000);
  await client.waitFor(`(() => {
    const [launch, join] = document.querySelectorAll('.profile-row:nth-child(2) .profile-row__buttons button');
    return launch?.textContent.trim() === 'Launch' && !launch.disabled
      && join?.textContent.trim() === 'Join' && join.disabled;
  })()`, 'ready profile with Join disabled until a server is available', 20_000);
  if (!(await client.evaluate(noSyncOnCard(secondProfile)))) {
    throw new Error('An up-to-date profile should not show Sync on its card');
  }
  await client.capture('02g-profiles-launch.png');

  const installedFile = path.join(secondProfileRoot, fixtureMod, 'addons', 'hello.txt');
  if (!fixtureBytes.equals(await readFile(installedFile))) {
    throw new Error('Validate files should install files for an unsynced profile');
  }
  await writeFile(installedFile, Buffer.alloc(fixtureBytes.length, 120));
  await validateFromMenu(secondProfile);
  await client.waitFor(`document.querySelector('.profile-row:nth-child(2) .profile-row__buttons .btn--primary')?.textContent.trim() === 'Launch'`, 'validation repaired the modified file', 20_000);
  if (!fixtureBytes.equals(await readFile(installedFile))) {
    throw new Error('Validate files should repair modified files');
  }

  const primaryAction = `document.querySelector('.profile-row:nth-child(2) .profile-row__buttons .btn--primary')?.textContent.trim()`;
  await rm(installedFile);
  await checkForUpdates(secondProfile);
  await client.waitFor(`${primaryAction} === 'Repair'`, 'repair offered for missing local content', 20_000);
  await client.capture('02p-profiles-repair.png');
  await client.clickText('Repair', secondProfile);
  await client.waitFor(`${primaryAction} === 'Launch'`, 'repair finished', 20_000);
  if (!fixtureBytes.equals(await readFile(installedFile))) {
    throw new Error('Repair should restore the missing file');
  }

  repo.serveManifest([{ name: 'Test server', address: '127.0.0.1', port: 2302, password: '', battleEye: false }]);
  await checkForUpdates(secondProfile);
  await client.waitFor(`${primaryAction} === 'Update'`, 'update offered for a newer repository revision', 20_000);
  await client.capture('02h-profiles-update.png');
  await client.clickText('Update', secondProfile);
  await client.waitFor(`(() => {
    const buttons = [...document.querySelectorAll('.profile-row:nth-child(2) .profile-row__buttons button')];
    return !document.querySelector('.profile-operation')
      && buttons.map(button => button.textContent.trim()).join(',') === 'Launch,Join'
      && buttons.every(button => !button.disabled);
  })()`, 'ready profile with a server', 20_000);
  await client.capture('02i-profiles-launch-join.png');

  for (let remaining = 2; remaining > 0; remaining--) {
    await openActions();
    await client.clickText('Delete profile', '.profile-actions:popover-open');
    await client.clickText('Delete', modal);
    await client.waitFor(`document.querySelectorAll('.profile-row').length === ${remaining - 1}`, 'profile removed');
  }
  await client.waitFor(`document.querySelector('.profiles-page__empty')?.innerText.trim() === 'New profile'`, 'empty profiles shows only the creation action');
  await client.capture('02c-profiles-empty.png');
  await client.clickText('New profile');
  await client.setInput('.form-field:nth-child(1) .field__input', 'Neptune Unit');
  await client.setInput('.form-field:nth-child(2) .field__input', repo.unavailableSource);
  await client.setInput(folderInput, installFolder);
  await client.clickText('Create');
  await client.waitFor(`document.querySelector('.profile-row__name')?.textContent.trim() === 'Neptune Unit'`, 'created profile on Profiles');
  if (!(await client.evaluate(noSyncOnCard(firstProfile)))) {
    throw new Error('A new profile should only offer file validation in the menu');
  }
  await client.waitFor(`document.querySelector('.profile-row [popovertarget]')`, 'new profile check finished');
  await openActions();
  await client.clickText('Edit profile', '.profile-actions:popover-open');
  await client.waitFor(`[...document.querySelectorAll('.form-field .field__input')].some(input => input.value === ${JSON.stringify(installFolder)})`, 'saved custom mod install folder');
  await client.clickText('Cancel', '.page-footer');
  await client.waitFor(`document.querySelector('.profiles-page')`, 'profiles before factory reset');
  await client.click('[aria-label="Settings"]');
  await client.waitFor(`document.querySelector('.section__title')?.textContent.trim() === 'Updates'`, 'settings before factory reset');
  await openFactoryReset();
  await client.clickText('Factory reset', modal);
  await client.waitFor(`document.querySelector('input[aria-label="Arma 3 Directory"]') && !document.querySelector('.confirm-dialog:modal')`, 'factory reset returns to setup');
  await client.setInput('input[aria-label="Arma 3 Directory"]', exampleGameDirectory);
  await client.clickText('Continue');
  await client.waitFor(`document.querySelector('.profiles-page__empty')`, 'factory reset cleared profiles');


}

await ensureCdpPortAvailable();
const heldResponses = new Set();
const heldDownloads = new Set();
const fixtureBytes = Buffer.from('fleet dummy file\n');
const fixtureMod = '@fleet-example';
const fixtureModChecksum = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const digest = (data) => createHash('md5').update(data).digest('hex').toUpperCase();
const fixturePart = {
  Path: `addons/hello.txt_${fixtureBytes.length}`,
  Length: fixtureBytes.length,
  Start: 0,
  Checksum: digest(fixtureBytes),
};
const modManifest = JSON.stringify({
  Name: fixtureMod,
  Checksum: fixtureModChecksum,
  Files: [{
    Path: 'addons\\hello.txt',
    Length: fixtureBytes.length,
    Checksum: digest(fixturePart.Checksum),
    Parts: [fixturePart],
  }],
});
let repoManifest;
let downloadReleased = false;
const slowRepo = createHttpServer((request, response) => {
  if (request.url === '/test/repo.json') {
    response.writeHead(200, { 'Content-Type': 'application/json' })
      .end(JSON.stringify({ repoName: 'Test modpack', checksum: '0'.repeat(40), requiredMods: [] }));
    return;
  }
  if (request.url === '/invalid/repo.json') {
    response.writeHead(200, { 'Content-Type': 'application/json' }).end('{}');
    return;
  }
  if (request.url === '/unavailable/repo.json') {
    response.writeHead(404).end();
    return;
  }
  if (repoManifest) {
    if (request.url === '/repo.json' || request.url === `/${fixtureMod}/mod.srf`) {
      response.writeHead(200, { 'Content-Type': 'application/json' })
        .end(request.url === '/repo.json' ? repoManifest : modManifest);
    } else if (request.url === `/${fixtureMod}/addons/hello.txt`) {
      if (downloadReleased) {
        sendFixtureDownload(request, response);
      } else {
        const pending = { request, response };
        heldDownloads.add(pending);
        response.on('close', () => heldDownloads.delete(pending));
      }
    } else {
      response.writeHead(404).end();
    }
    return;
  }
  heldResponses.add(response);
  response.on('close', () => heldResponses.delete(response));
});

function sendFixtureDownload(request, response) {
  const range = /^bytes=(\d+)-(\d+)$/.exec(request.headers.range ?? '');
  if (range) {
    const start = Number(range[1]);
    const end = Number(range[2]);
    response.writeHead(206, {
      'Content-Range': `bytes ${start}-${end}/${fixtureBytes.length}`,
      'Content-Length': end - start + 1,
    })
      .end(fixtureBytes.subarray(start, end + 1));
  } else {
    response.writeHead(200, { 'Content-Length': fixtureBytes.length })
      .end(request.method === 'HEAD' ? undefined : fixtureBytes);
  }
}
await new Promise((resolve, reject) => {
  slowRepo.once('error', reject);
  slowRepo.listen(0, cdpHost, resolve);
});
await seedDummyConfig(slowRepo.address().port);

const child = spawn(executable, ['--config-dir', configRoot], {
  cwd: repoRoot,
  env: {
    ...process.env,
    WEBVIEW2_USER_DATA_FOLDER: path.join(testRoot, 'webview2'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
  },
  stdio: 'ignore',
});

let client;
try {
  client = await CdpClient.connect(await waitForTarget());
  await runFlow(client, {
    unavailableSource: `http://${cdpHost}:${slowRepo.address().port}/unavailable/repo.json`,
    testSource: `http://${cdpHost}:${slowRepo.address().port}/test/repo.json`,
    invalidManifestSource: `http://${cdpHost}:${slowRepo.address().port}/invalid/repo.json`,
    failPendingRequests() {
      for (const response of heldResponses) response.writeHead(404).end();
    },
    serveManifest(servers) {
      repoManifest = JSON.stringify({
        repoName: 'Fleet UI fixture',
        checksum: servers.length ? '1'.repeat(40) : '0'.repeat(40),
        requiredMods: [{ modName: fixtureMod, checkSum: fixtureModChecksum, enabled: true }],
        optionalMods: [],
        servers,
      });
    },
    releaseDownload() {
      downloadReleased = true;
      for (const { request, response } of heldDownloads) sendFixtureDownload(request, response);
    },
  });
  process.stdout.write(`Rendered Fleet UI views to ${captureRoot}\n`);
} finally {
  client?.close();
  child.kill();
  slowRepo.closeAllConnections();
  slowRepo.close();
}

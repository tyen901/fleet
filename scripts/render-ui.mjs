import { spawn } from 'node:child_process';
import { mkdir, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const testRoot = path.resolve(
  process.env.FLEET_UI_TEST_ROOT ?? path.join(repoRoot, 'target', 'ui-render'),
);
const configRoot = path.join(testRoot, 'config');
const captureRoot = path.join(testRoot, 'captures');
const profileRoot = path.join(testRoot, 'profile');
const cdpPort = Number(process.env.FLEET_UI_TEST_CDP_PORT ?? 9333);
const cdpHost = '127.0.0.1';
const previewWidth = 420;
const previewHeight = 560;
const executable = path.join(
  repoRoot,
  'target',
  'debug',
  process.platform === 'win32' ? 'fleet.exe' : 'fleet',
);

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

if (!Number.isInteger(cdpPort) || cdpPort < 1 || cdpPort > 65_535) {
  throw new Error('FLEET_UI_TEST_CDP_PORT must be an integer from 1 to 65535');
}

function ensureCdpPortAvailable() {
  const server = createServer();
  return new Promise((resolve, reject) => {
    server.once('error', (error) => {
      if (error?.code === 'EADDRINUSE') {
        reject(
          new Error(
            `Fleet UI renderer cannot use CDP port ${cdpPort}: it is already in use. Set FLEET_UI_TEST_CDP_PORT to an unused port and rerun.`,
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

  async clickText(label) {
    const clicked = await this.evaluate(`(() => {
      const button = [...document.querySelectorAll('button')]
        .find((candidate) => candidate.textContent.trim() === ${JSON.stringify(label)} && !candidate.disabled);
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
    await delay(150);
    const controls = await this.evaluate(`(() => {
      // The footer's back affordance is a named exception: icon+label in one
      // control, so it carries its own (tighter) padding, not the shared 12px.
      const buttons = [...document.querySelectorAll('.btn')]
        .filter((button) => !button.classList.contains('page-footer__back'));
      const fields = [...document.querySelectorAll('.field__input, .select')];
      return {
        buttonsConsistent: buttons.every((button) => {
          const style = getComputedStyle(button);
          return button.getBoundingClientRect().height === 34 &&
            style.paddingLeft === '12px' &&
            style.paddingRight === '12px' &&
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

async function seedDummyConfig() {
  await Promise.all(
    [configRoot, profileRoot].map((directory) => rm(directory, { recursive: true, force: true })),
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
        source: 'http://127.0.0.1:9/repo.json',
        destination: profileRoot,
        arma3_server: null,
        swifty_repo_revision: '',
        launch_params: '',
        additional_mod_folders: ['@ace', '@cba_a3'],
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

async function runFlow(client) {
  await client.call('Runtime.enable');
  await client.call('Page.enable');
  await client.call('Emulation.setDeviceMetricsOverride', {
    width: previewWidth,
    height: previewHeight,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await client.waitFor(`document.body?.innerText.includes('FLEET')`, 'onboarding view');

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
  await client.setInput('input[type="text"]', path.join(testRoot, 'arma3'));
  await client.clickText('Continue');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');

  const cardState = await client.evaluate(`(() => {
    const card = document.querySelector('.profile-row');
    const footer = document.querySelector('.page-footer');
    const newProfile = footer.querySelector('[aria-label="New profile"]');
    const settings = footer.querySelector('[aria-label="Settings"]');
    const buttons = [...card.querySelectorAll('.profile-row__buttons button')]
      .map((button) => button.textContent.trim());
    return {
      text: card.textContent,
      buttons,
      mainTag: card.querySelector('.profile-row__main').tagName,
      direction: getComputedStyle(card.querySelector('.profile-row__buttons')).flexDirection,
      hasSettingsAction: Boolean(card.querySelector('[aria-label="Profile details"]')),
      newProfileLeft: newProfile.getBoundingClientRect().left,
      settingsLeft: settings.getBoundingClientRect().left,
      newProfileWidth: newProfile.getBoundingClientRect().width,
      settingsWidth: settings.getBoundingClientRect().width,
      navIsIconOnly: !newProfile.textContent.trim() && !settings.textContent.trim(),
    };
  })()`);
  if (
    cardState.text.includes(profileRoot) ||
    cardState.text.includes('Last checked') ||
    cardState.text.includes('Status unknown')
  ) {
    throw new Error('Profile card exposes local path, last-checked, or a non-actionable status');
  }
  if (
    cardState.buttons.join(',') !== 'Launch,Join' ||
    cardState.direction !== 'row' ||
    cardState.mainTag !== 'DIV' ||
    !cardState.hasSettingsAction ||
    cardState.newProfileLeft >= cardState.settingsLeft ||
    cardState.newProfileWidth !== cardState.settingsWidth ||
    !cardState.navIsIconOnly
  ) {
    throw new Error('Profile card controls do not match the expected explicit-action layout');
  }
  await client.capture('02-profiles.png');

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
      .find((row) => row.querySelector('.field-row__title')?.textContent.trim() === 'Game directory');
    const resetSlots = [...document.querySelectorAll('.field-reset')];
    const cancel = [...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Cancel');
    const toggles = [...document.querySelectorAll('.field-row input[type="checkbox"]')];
    const save = [...document.querySelectorAll('.page-footer button')]
      .find((button) => button.textContent.trim() === 'Save');
    return {
      sectionTitles,
      gameDirectoryButtons: gameRow.querySelectorAll('button').length,
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
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 100000`);
  await delay(100);
  await client.capture('04-settings-bottom.png');
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
  await client.capture('05-new-profile.png');
  await client.clickText('Cancel');
  await client.waitFor(`document.querySelector('.profiles-page__list')`, 'profiles view');

  await client.click('[aria-label="Profile details"]');
  await client.waitFor(`document.body.innerText.includes('Validate')`, 'profile overview');
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .some((button) => button.textContent.trim() === 'Cancel')`,
    'profile overview cancel action',
  );
  const profileOverviewLayout = await client.evaluate(`(() => {
    const syncSection = [...document.querySelectorAll('.section')]
      .find((section) => section.querySelector('.section__title')?.textContent.trim() === 'Sync');
    return {
      hasNoHeader: document.querySelectorAll('.page-header').length === 0,
      hasReadyState: document.body.innerText.includes('Ready to play'),
      hasLaunchOrJoin: [...document.querySelectorAll('button')]
        .some((button) => ['Launch', 'Join'].includes(button.textContent.trim())),
      syncActions: [...syncSection.querySelectorAll('.field-row__title')]
        .map((heading) => heading.textContent.trim()),
      // Read mode uses the real controls, locked rather than replaced.
      readonlyInputs: [...document.querySelectorAll('.form-field .field__input')]
        .every((input) => input.readOnly),
      readonlyInputCount: document.querySelectorAll('.form-field .field__input').length,
      // An inline row is one control tall whether or not it holds a button.
      inlineRowHeights: [
        ...new Set(
          [...document.querySelectorAll('.field-row')]
            .filter((row) => row.querySelector('.field-row__control--actions'))
            .map((row) => Math.round(row.getBoundingClientRect().height)),
        ),
      ],
      // Buttons are filled rather than outlined; every variant shares one
      // border treatment (none) so state reads from fill/text color alone.
      buttonsBorderless: [...document.querySelectorAll('.btn')].every(
        (button) => getComputedStyle(button).borderTopColor === 'rgba(0, 0, 0, 0)',
      ),
      // Inputs use a single border color regardless of read-only/disabled
      // state; only the text communicates emptiness.
      inputBordersConsistent: new Set(
        [...document.querySelectorAll('.field__input')].map(
          (input) => getComputedStyle(input).borderTopColor,
        ),
      ).size <= 1,
      mods: [...document.querySelectorAll('.mod-list__item')].map((item) => item.textContent.trim()),
    };
  })()`);
  if (
    !profileOverviewLayout.hasNoHeader ||
    profileOverviewLayout.hasReadyState ||
    profileOverviewLayout.hasLaunchOrJoin ||
    profileOverviewLayout.syncActions.join(',') !==
      'Check profile,Sync profile,Validate local files' ||
    !profileOverviewLayout.readonlyInputs ||
    profileOverviewLayout.readonlyInputCount !== 4 ||
    profileOverviewLayout.inlineRowHeights.join(',') !== '34' ||
    !profileOverviewLayout.inputBordersConsistent ||
    !profileOverviewLayout.buttonsBorderless ||
    profileOverviewLayout.mods.join(',') !== '@ace,@cba_a3'
  ) {
    throw new Error('Profile overview header, Sync section, or additional-mod list is incorrect');
  }
  await client.capture('06-profile-overview.png');
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 100000`);
  await delay(100);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .some((button) => button.textContent.trim() === 'Cancel')`,
    'scrolled profile overview cancel action',
  );
  await client.capture('06-profile-overview-mods.png');

  await client.clickText('Edit');
  await client.waitFor(`document.body.innerText.includes('PROFILE REMOVAL')`, 'profile edit mode');
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 0`);
  await delay(100);
  await client.waitFor(
    `[...document.querySelectorAll('.page-footer button')]
      .some((button) => button.textContent.trim() === 'Cancel')`,
    'edit profile cancel action',
  );
  await client.capture('07-edit-profile-top.png');
  await client.evaluate(`(() => {
    document.querySelector('.mod-list').closest('.section').scrollIntoView({ block: 'start' });
  })()`);
  await delay(100);
  await client.capture('08-edit-profile-mods.png');

  const editButtons = await client.evaluate(`(() => {
    const modRows = [...document.querySelectorAll('.mod-list__row')];
    const labels = modRows.flatMap((row) => [...row.querySelectorAll('button')]
      .map((button) => button.textContent.trim()));
    const compared = ['Select', 'Open', 'Browse', 'Remove']
      .map((label) => [...document.querySelectorAll('button')]
        .find((button) => button.textContent.trim() === label))
      .filter(Boolean)
      .map((button) => {
        const style = getComputedStyle(button);
        return [style.fontFamily, style.fontSize, style.fontWeight, style.letterSpacing, style.textTransform].join('|');
      });
    const additionalMods = document.querySelector('.mod-list').closest('.section');
    const addButton = additionalMods.querySelector('[aria-label="Add mod"]');
    const list = additionalMods.querySelector('.mod-list');
    const footerLabels = [...document.querySelectorAll('.page-footer button')]
      .map((button) => button.getAttribute('aria-label') ?? button.textContent.trim());
    return {
      labels,
      sharedTypography: compared.length === 4 && new Set(compared).size === 1,
      addBeforeList: addButton.getBoundingClientRect().bottom <= list.getBoundingClientRect().top,
      // Right aligned above the list.
      addRightAligned:
        Math.abs(
          addButton.getBoundingClientRect().right -
            additionalMods.getBoundingClientRect().right,
        ) < 1,
      // There is no back affordance anywhere; Cancel is the only way out.
      hasBack: footerLabels.includes('Back'),
      hasCancel: footerLabels.includes('Cancel'),
    };
  })()`);
  if (
    editButtons.labels.filter((label) => label === 'Browse').length !== 2 ||
    editButtons.labels.filter((label) => label === 'Remove').length !== 2 ||
    !editButtons.sharedTypography ||
    !editButtons.addBeforeList ||
    !editButtons.addRightAligned ||
    editButtons.hasBack ||
    !editButtons.hasCancel
  ) {
    throw new Error('Browse controls or shared button typography are incorrect');
  }

  const originalModCount = await client.evaluate(
    `document.querySelectorAll('.mod-list__row').length`,
  );
  await client.click('[aria-label="Add mod"]');
  await client.waitFor(
    `document.querySelectorAll('.mod-list__row').length === ${originalModCount + 1}`,
    'new additional-mod row',
  );
  await client.evaluate(`(() => {
    document.querySelector('.mod-list').closest('.section').scrollIntoView({ block: 'start' });
  })()`);
  await delay(100);
  await client.capture('09-edit-profile-mod-added.png');
  await client.evaluate(`(() => {
    const buttons = [...document.querySelectorAll('.mod-list__row button')]
      .filter((button) => button.textContent.trim() === 'Remove');
    buttons.at(-1).click();
  })()`);
  await client.waitFor(
    `document.querySelectorAll('.mod-list__row').length === ${originalModCount}`,
    'additional-mod row removal',
  );
  await client.evaluate(`document.querySelector('.page-frame__body').scrollTop = 100000`);
  await delay(100);
  await client.clickText('Delete');
  await client.waitFor(`document.querySelector('.inline-confirm')`, 'delete confirmation');
  await delay(600);
  await client.capture('12-delete-confirm.png');
  await client.evaluate(`(() => {
    [...document.querySelectorAll('.inline-confirm button')]
      .find((button) => button.textContent.trim() === 'Cancel').click();
  })()`);

  await client.clickText('Cancel');
  await client.waitFor(`document.body.innerText.includes('Validate')`, 'profile read mode');

  await client.clickText('Sync');
  await client.waitFor(`document.querySelector('.sync-panel')`, 'sync progress', 15_000);
  await client.waitFor(
    `document.querySelector('.sync-panel__phase')?.textContent.trim() === 'Hashing local files' &&
      document.querySelector('.sync-panel__count')?.textContent.includes('files') &&
      document.body.innerText.includes('MiB/s') &&
      document.body.innerText.includes('About')`,
    'inventory rebuild rate and remaining time',
    15_000,
  );
  await client.capture('10-inventory-rebuild-progress.png');
  // FLEET_SIMULATE_SYNC drives a scripted sequence that parks at
  // FLEET_SIMULATE_SYNC_HOLD_PERCENT, so this capture is reproducible and no
  // content is actually downloaded.
  await client.waitFor(
    `document.querySelector('.sync-panel__phase')?.textContent.trim() === 'Syncing files' &&
      document.querySelector('.sync-panel__percent')?.textContent.trim() === '50%'`,
    'simulated sync at 50%',
    15_000,
  );
  await client.capture('10-sync-progress.png');
  await client.clickText('Cancel');
  await client.waitFor(
    `document.body.innerText.includes('Stopping sync') &&
      [...document.querySelectorAll('button')]
        .some((button) => button.textContent.trim() === 'Stopping')`,
    'immediate stopping state',
  );
  await client.capture('11-sync-stopping.png');
}

await ensureCdpPortAvailable();
await seedDummyConfig();

const child = spawn(executable, [], {
  cwd: repoRoot,
  env: {
    ...process.env,
    FLEET_CONFIG_DIR: configRoot,
    FLEET_SIMULATE_SYNC: '1',
    FLEET_SIMULATE_SYNC_HOLD_PERCENT: '50',
    WEBVIEW2_USER_DATA_FOLDER: path.join(testRoot, 'webview2'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
  },
  stdio: 'ignore',
});

let client;
try {
  client = await CdpClient.connect(await waitForTarget());
  await runFlow(client);
  process.stdout.write(`Rendered Fleet UI views to ${captureRoot}\n`);
} finally {
  client?.close();
  child.kill();
}

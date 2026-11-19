// @ts-check

// This ugly nonsense is how you express `as const` in JSDoc. Eww. Is this worth avoiding
// a build step? Maaaaaybe?
const Level = /** @type {const} */ ({
   Off: 0,
   Error: 1,
   Warn: 2,
   Info: 3,
   Debug: 4,
   Trace: 5,
});

/** @type {Array<number>} */
const LEVEL_VALUES = Object.values(Level);

/** @typedef {typeof Level[keyof typeof Level]} Level */

// Because if `@types/node` ends up anywhere in the program somehow, this won’t compile
// correctly if it says “number”. Annoyingly. difficult to fix in a JS-only setup.
/** @type {ReturnType<typeof setInterval> | null} */
let reconnectInterval = null;

/** @type {WebSocket | null} */
let ws = null;

connect();

function connect() {
   const endpoint = new URL('/live-reload', location.href);
   endpoint.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
   ws = new WebSocket(endpoint);
   debug('created WebSocket instance');

   ws.addEventListener('open', (event) => {
      debug('WebSocket connection opened');
      if (reconnectInterval) {
         clearInterval(reconnectInterval);
         reconnectInterval = null;
      }
   });

   ws.addEventListener('error', (event) => {
      error('error:', event);
   });

   ws.addEventListener('close', (closeEvent) => {
      let message = `closed because ${closeEvent.reason}`;
      if (closeEvent.wasClean) {
         debug(message);
      } else {
         // might handle this distinctively for expected closures.
         error(message, closeEvent.code);
      }

      if (!reconnectInterval) {
         info('connection closed');
         reconnectInterval = setInterval(() => {
            debug('attempting to reconnect…');
            connect();
         }, 5_000);
      }
   });

   ws.addEventListener('message', (message) => {
      console.log('got message', message);
      // TODO:
      //
      // 1. Update the Serde config to send this as `type: 'Reload', values: ...`.
      // 2. Only reload if the path is relevant.
      let payload;
      try {
         payload = JSON.parse(message.data);
         debug('Parsed JSON payload of WebSocket message', payload);
      } catch (e) {
         error('Error deserializing WebSocket message', e);
      }

      if (payload.Reload) {
         debug('Specified reload paths were:', message.data.Reload);
         location.reload();
      }
   });

   debug('added all WebSocket listeners');
}

const LS_KEY = 'lx:logLevel';

/** @param {Level} value */
function setLogLevel(value) {
   globalThis.localStorage.setItem(LS_KEY, value.toString());
}

function resetLogLevel() {
   globalThis.localStorage.removeItem(LS_KEY);
}

/**
  @param {Array<unknown>}
  @returns {void}
 */
function error(...messages) {
   if (logLevel() >= Level.Error) console.error('[Error]', ...messages);
}

/**
  @param {Array<unknown>}
  @returns {void}
 */
function warn(...messages) {
   if (logLevel() >= Level.Warn) console.warn('[Warning]', ...messages);
}

/**
  @param {Array<unknown>}
  @returns {void}
 */
function info(...messages) {
   if (logLevel() >= Level.Info) console.info('[Info]', ...messages);
}

/**
  @param {Array<unknown>}
  @returns {void}
 */
function debug(...messages) {
   if (logLevel() >= Level.Debug) console.info('[Debug]', ...messages);
}

/**
  @param {Array<unknown>}
  @returns {void}
 */
function trace(...messages) {
   if (logLevel() >= Level.Debug) console.info('[Trace]', ...messages);
}

/**
  Read the log level from local storage, or fall back to the default of `Info`.
  @returns {Level}
 */
function logLevel() {
   const stored = globalThis.localStorage.getItem(LS_KEY);
   if (stored == null) {
      return Level.Info;
   }

   const parsed = Number(stored);
   if (!isLogLevel(parsed)) {
      console.error('Ignoring, and purging, invalid log level from storage', parsed);
      resetLogLevel();
      return Level.Info;
   }

   return parsed;
}

/**
  @param {number} value
  @returns {value is Level}
 */
function isLogLevel(value) {
   return LEVEL_VALUES.includes(value);
}

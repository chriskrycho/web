/**
   @module live-reload

   A simple live-reload script. Types are currently manually synced with the back end.
  
   Since this emits ESM, there is no need to worry about the old IIFE-type dynamics:
   nothing here is global.
 */

const Level = {
   Off: 0,
   Error: 1,
   Warn: 2,
   Info: 3,
   Debug: 4,
   Trace: 5,
} as const;

type Level = (typeof Level)[keyof typeof Level];

const LEVEL_VALUES: Array<number> = Object.values(Level);

const LS_KEY = 'lx:logLevel';

// Because if `@types/node` ends up anywhere in the program somehow, this won’t compile
// correctly if it says “number”. Annoyingly. difficult to fix in a JS-only setup.
type Interval = ReturnType<typeof setInterval>;

let reconnectInterval: Interval | null = null;

let ws: WebSocket | null = null;

connect();

function connect() {
   const endpoint = new URL('/live-reload', location.href);
   endpoint.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
   ws = new WebSocket(endpoint);
   debug('created WebSocket instance');

   ws.addEventListener('open', (event) => {
      debug('WebSocket connection opened');
      trace(event);
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
      trace(closeEvent);

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

function setLogLevel(value: Level) {
   globalThis.localStorage.setItem(LS_KEY, value.toString());
}

function resetLogLevel() {
   globalThis.localStorage.removeItem(LS_KEY);
}

function error(...messages: Array<unknown>) {
   if (logLevel() >= Level.Error) console.error('[Error]', ...messages);
}

function warn(...messages: Array<unknown>) {
   if (logLevel() >= Level.Warn) console.warn('[Warning]', ...messages);
}

function info(...messages: Array<unknown>) {
   if (logLevel() >= Level.Info) console.info('[Info]', ...messages);
}

function debug(...messages: Array<unknown>) {
   if (logLevel() >= Level.Debug) console.info('[Debug]', ...messages);
}

function trace(...messages: Array<unknown>) {
   if (logLevel() >= Level.Debug) console.info('[Trace]', ...messages);
}

function logLevel(): Level {
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

function isLogLevel(value: number): value is Level {
   return LEVEL_VALUES.includes(value);
}

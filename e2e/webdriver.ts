//! A minimal W3C WebDriver client — no dependency, `fetch` and nothing else.
//!
//! WebdriverIO would work too, but it drags a hundred packages into a project
//! whose whole test story so far has been "npx tsx, nothing installed". The
//! protocol we actually need is nine endpoints wide, so it lives here in full
//! sight rather than behind a framework.

/// The W3C element identifier — a magic string, not a convention we chose.
const ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';

export class WebDriverError extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = 'WebDriverError';
  }
}

/// Every request is bounded. A WebDriver call that never returns is the worst
/// failure mode available here: it hangs the suite with no output at all, and
/// this project has paid for that lesson twice (pitfalls 39 and 40).
const CALL_TIMEOUT_MS = 120_000;

async function call(
  base: string,
  method: 'GET' | 'POST' | 'DELETE',
  path: string,
  body?: unknown,
): Promise<any> {
  let res: Response;
  try {
    res = await fetch(`${base}${path}`, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(CALL_TIMEOUT_MS),
    });
  } catch (e) {
    const why = e instanceof Error ? e.message : String(e);
    throw new WebDriverError('transport', `${method} ${path} a échoué : ${why}`);
  }
  const text = await res.text();
  let parsed: any;
  try {
    parsed = text ? JSON.parse(text) : {};
  } catch {
    throw new WebDriverError('invalid response', `${method} ${path} → ${res.status}: ${text.slice(0, 300)}`);
  }
  if (!res.ok) {
    const err = parsed?.value ?? {};
    throw new WebDriverError(err.error ?? String(res.status), err.message ?? text.slice(0, 300));
  }
  return parsed.value;
}

export class Element {
  constructor(
    private readonly session: Session,
    readonly id: string,
  ) {}

  click(): Promise<void> {
    return this.session.post(`/element/${this.id}/click`, {}).then(() => undefined);
  }

  clear(): Promise<void> {
    return this.session.post(`/element/${this.id}/clear`, {}).then(() => undefined);
  }

  sendKeys(text: string): Promise<void> {
    return this.session.post(`/element/${this.id}/value`, { text }).then(() => undefined);
  }

  text(): Promise<string> {
    return this.session.get(`/element/${this.id}/text`);
  }

  attribute(name: string): Promise<string | null> {
    return this.session.get(`/element/${this.id}/attribute/${name}`);
  }

  property(name: string): Promise<any> {
    return this.session.get(`/element/${this.id}/property/${name}`);
  }

  cssValue(name: string): Promise<string> {
    return this.session.get(`/element/${this.id}/css/${name}`);
  }

  displayed(): Promise<boolean> {
    return this.session.get(`/element/${this.id}/displayed`);
  }

  /// Search inside this element rather than the whole document.
  async find(selector: string): Promise<Element> {
    const value = await this.session.post(`/element/${this.id}/element`, {
      using: 'css selector',
      value: selector,
    });
    return new Element(this.session, value[ELEMENT_KEY]);
  }

  async findAll(selector: string): Promise<Element[]> {
    const value = await this.session.post(`/element/${this.id}/elements`, {
      using: 'css selector',
      value: selector,
    });
    return value.map((v: any) => new Element(this.session, v[ELEMENT_KEY]));
  }
}

export class Session {
  private constructor(
    private readonly base: string,
    readonly id: string,
  ) {}

  static async create(base: string, capabilities: Record<string, unknown>): Promise<Session> {
    const value = await call(base, 'POST', '/session', {
      capabilities: { alwaysMatch: capabilities },
    });
    // Some drivers answer with the session id nested, some flat.
    const id = value.sessionId ?? value.value?.sessionId;
    if (!id) throw new WebDriverError('no session', `driver returned no sessionId: ${JSON.stringify(value).slice(0, 300)}`);
    return new Session(base, id);
  }

  get(path: string): Promise<any> {
    return call(this.base, 'GET', `/session/${this.id}${path}`);
  }

  post(path: string, body: unknown): Promise<any> {
    return call(this.base, 'POST', `/session/${this.id}${path}`, body);
  }

  async find(selector: string): Promise<Element> {
    const value = await this.post('/element', { using: 'css selector', value: selector });
    return new Element(this, value[ELEMENT_KEY]);
  }

  async findAll(selector: string): Promise<Element[]> {
    const value = await this.post('/elements', { using: 'css selector', value: selector });
    return value.map((v: any) => new Element(this, v[ELEMENT_KEY]));
  }

  /// `null` rather than a throw — for "is it gone?" assertions.
  async maybeFind(selector: string): Promise<Element | null> {
    try {
      return await this.find(selector);
    } catch (e) {
      if (e instanceof WebDriverError && e.code === 'no such element') return null;
      throw e;
    }
  }

  /// Poll until `selector` matches, or fail with a message naming the selector.
  /// Every UI assertion in this suite goes through a wait: the app mounts
  /// asynchronously and a bare `find` would race it.
  async waitFor(selector: string, timeoutMs = 10_000): Promise<Element> {
    const deadline = Date.now() + timeoutMs;
    let last: unknown;
    for (;;) {
      try {
        const el = await this.find(selector);
        if (await el.displayed()) return el;
      } catch (e) {
        last = e;
      }
      if (Date.now() > deadline) {
        throw new Error(
          `délai dépassé (${timeoutMs} ms) en attendant « ${selector} »` +
            (last instanceof Error ? ` — dernière erreur : ${last.message}` : ''),
        );
      }
      await new Promise((r) => setTimeout(r, 120));
    }
  }

  /// Poll until `predicate` holds. `label` is what the failure message says.
  async waitUntil(label: string, predicate: () => Promise<boolean>, timeoutMs = 10_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      let ok = false;
      try {
        ok = await predicate();
      } catch {
        ok = false;
      }
      if (ok) return;
      if (Date.now() > deadline) {
        throw new Error(`délai dépassé (${timeoutMs} ms) en attendant : ${label}`);
      }
      await new Promise((r) => setTimeout(r, 120));
    }
  }

  /// `script` runs in the page. Return a value to get it back.
  execute<T = any>(script: string, args: unknown[] = []): Promise<T> {
    return this.post('/execute/sync', { script, args });
  }

  /// Async variant: the last argument passed to `script` is a callback, and
  /// the value handed to it is what this resolves with. This is the only way
  /// to await a Tauri `invoke()` from a test — `withGlobalTauri: true` puts
  /// the IPC on `window.__TAURI__`, and it is promise-based.
  executeAsync<T = any>(script: string, args: unknown[] = []): Promise<T> {
    return this.post('/execute/async', { script, args });
  }

  /// Call a Tauri command and get its result, or `"ERR:<message>"` on reject.
  /// Errors come back as a value rather than a throw so a failing command
  /// produces a readable assertion instead of a WebDriver stack trace.
  invoke<T = any>(command: string, payload: Record<string, unknown> = {}): Promise<T> {
    return this.executeAsync<T>(
      `const done = arguments[arguments.length - 1];
       window.__TAURI__.core.invoke(arguments[0], arguments[1])
         .then((r) => done(r))
         .catch((e) => done('ERR:' + String(e)));`,
      [command, payload],
    );
  }

  /// Keyboard input at the window level (not aimed at an element) — this is
  /// what exercises the global shortcuts of LOT-20.
  async keys(keys: string[]): Promise<void> {
    const actions = keys.flatMap((key) => [
      { type: 'keyDown', value: key },
      { type: 'keyUp', value: key },
    ]);
    // A chord is expressed as nested downs then ups, so callers pass the
    // modifier first and we reverse the ups.
    await this.post('/actions', {
      actions: [{ type: 'key', id: 'keyboard', actions }],
    });
  }

  /// A real chord: every key down in order, then up in reverse order.
  /// Move the pointer over an element and leave it there.
  ///
  /// `lib/tooltip.ts` listens for `pointerover`, so a synthetic event dispatched
  /// from `execute` would test our own dispatch rather than the app's wiring.
  /// This drives the real pointer through the driver, which is the point.
  async hover(element: Element): Promise<void> {
    await this.post('/actions', {
      actions: [
        {
          type: 'pointer',
          id: 'mouse',
          parameters: { pointerType: 'mouse' },
          actions: [
            { type: 'pointerMove', duration: 0, origin: { [ELEMENT_KEY]: element.id }, x: 0, y: 0 },
            { type: 'pause', duration: 60 },
          ],
        },
      ],
    });
  }

  /// Move the pointer to a fixed viewport position — used to leave an element.
  async pointerTo(x: number, y: number): Promise<void> {
    await this.post('/actions', {
      actions: [
        {
          type: 'pointer',
          id: 'mouse',
          parameters: { pointerType: 'mouse' },
          actions: [{ type: 'pointerMove', duration: 0, origin: 'viewport', x, y }],
        },
      ],
    });
  }

  async chord(keys: string[]): Promise<void> {
    const down = keys.map((value) => ({ type: 'keyDown', value }));
    const up = [...keys].reverse().map((value) => ({ type: 'keyUp', value }));
    await this.post('/actions', {
      actions: [{ type: 'key', id: 'keyboard', actions: [...down, ...up] }],
    });
  }

  async screenshot(): Promise<Buffer> {
    const b64 = await this.get('/screenshot');
    return Buffer.from(b64, 'base64');
  }

  async quit(): Promise<void> {
    try {
      await call(this.base, 'DELETE', `/session/${this.id}`);
    } catch {
      // A driver that already tore the session down is not a test failure.
    }
  }
}

/// W3C key codes used by the suite.
export const Key = {
  Control: '',
  Shift: '',
  Alt: '',
  Escape: '',
  Enter: '',
  Tab: '',
  Backspace: '',
  ArrowLeft: '',
  ArrowUp: '',
  ArrowRight: '',
  ArrowDown: '',
  Home: '',
  End: '',
  Space: '',
} as const;

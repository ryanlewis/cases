// A stand-in for preact/hooks, which composer.test.js gives compose.ts in
// place of preact's own, so that a test can call a component as a function,
// run its effects, call the handlers in the tree it gives, and call it again
// with the state they set. Nothing is put in a page: the test hands the
// component's refs the stand-in elements it wants them to hold.

let current = null;

// The component `component` with `props`, ready to draw. `attach(tree)`
// is called after each call of the component, before its effects run, to
// hand its refs their elements.
export function mount(component, props, attach = () => {}) {
  const self = {
    slots: [],
    effects: [],
    dirty: true,
    tree: null,
    // Calls the component again while its state has changed since, running
    // its effects after each call, and gives the tree it gave last.
    draw() {
      while (self.dirty) {
        self.dirty = false;
        self.effects = [];
        current = { self, at: 0 };
        try {
          self.tree = component(props);
        } finally {
          current = null;
        }
        attach(self.tree);
        for (const run of self.effects) run();
      }
      return self.tree;
    },
    // Runs the cleanups the effects left, as preact does when it unmounts.
    unmount() {
      for (const slot of self.slots) slot?.cleanup?.();
    },
  };
  self.draw();
  return self;
}

function slot() {
  if (!current) throw new Error("a hook called outside a component");
  return [current.self, current.at++];
}

export function useState(initial) {
  const [self, at] = slot();
  if (!(at in self.slots)) self.slots[at] = { value: typeof initial === "function" ? initial() : initial };
  const held = self.slots[at];
  // As preact's, a value the state has already draws nothing again.
  const set = (next) => {
    const value = typeof next === "function" ? next(held.value) : next;
    if (Object.is(value, held.value)) return;
    held.value = value;
    self.dirty = true;
  };
  return [held.value, set];
}

export function useRef(initial) {
  const [self, at] = slot();
  if (!(at in self.slots)) self.slots[at] = { current: initial };
  return self.slots[at];
}

export function useEffect(effect, deps) {
  const [self, at] = slot();
  const was = self.slots[at];
  if (was && deps && was.deps && deps.every((d, i) => Object.is(d, was.deps[i]))) return;
  self.effects.push(() => {
    was?.cleanup?.();
    const cleanup = effect();
    self.slots[at] = { deps, cleanup: typeof cleanup === "function" ? cleanup : undefined };
  });
  // The slot is taken until the effect runs.
  if (!was) self.slots[at] = { deps: undefined };
}

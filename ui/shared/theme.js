// Applies the theme and motion settings to the document. Event-driven: it
// listens to the OS media queries instead of polling.
import { config, cssEasing } from './spring.js';

const dark = matchMedia('(prefers-color-scheme: dark)');
const less = matchMedia('(prefers-reduced-motion: reduce)');
let current = { theme: 'system', motion: 'system', spring_feel: 'default' };
let onChange = () => {};

/** True when motion should be reduced to cross-fades. */
export function reducedMotion() {
  return current.motion === 'reduced' || (current.motion === 'system' && less.matches);
}

/** Spring parameters for a preset under the current settings. */
export function spring(name) {
  return config(name, current.spring_feel, reducedMotion());
}

function apply() {
  const root = document.documentElement;
  const isDark = current.theme === 'dark' || (current.theme === 'system' && dark.matches);
  root.dataset.theme = isDark ? 'dark' : 'light';
  root.dataset.motion = reducedMotion() ? 'reduced' : 'full';
  root.style.setProperty('--spring-snappy', cssEasing(spring('snappy')));
  root.style.setProperty('--spring-bouncy', cssEasing(spring('bouncy')));
  onChange();
}

/** Call with the `appearance` settings whenever they change. `changed` runs after every re-apply. */
export function setAppearance(appearance, changed) {
  current = appearance;
  if (changed) onChange = changed;
  apply();
}

dark.addEventListener('change', apply);
less.addEventListener('change', apply);

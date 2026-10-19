import { iconId, type IconName } from '~~/icons'

/** A Havre icon's name for Nuxt UI (`mail` → `i-havre-mail`), when it's chosen at run time. */
export function havreIcon(name: IconName) {
  return `i-havre-${iconId(name)}`
}

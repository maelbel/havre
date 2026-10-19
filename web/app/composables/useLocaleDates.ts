// The browser's language, for date and time fields: it decides the order
// of day and month and 12/24-hour times, as the native fields did.
export function browserLocale() {
  return typeof navigator !== 'undefined' && navigator.language ? navigator.language : 'en-GB'
}

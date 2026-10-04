/** FX pairs, built in so the dropdown needs no network on mount.
 *
 * The live vault catalog is 22,000+ rows across every asset class and only
 * ~60 of them are FX, so fetching it to fill this list was never worth it.
 * "Refresh from LSE" replaces this with live FX, commodities and indices. */
export const FX_PAIRS = [
  "EUR/USD", "GBP/USD", "USD/JPY", "USD/CHF", "USD/CAD", "AUD/USD", "NZD/USD",
  "EUR/GBP", "EUR/JPY", "EUR/CHF", "EUR/AUD", "EUR/CAD", "EUR/NZD",
  "GBP/JPY", "GBP/CHF", "GBP/AUD", "GBP/CAD", "GBP/NZD",
  "AUD/JPY", "AUD/CHF", "AUD/CAD", "AUD/NZD",
  "NZD/JPY", "NZD/CHF", "NZD/CAD",
  "CAD/JPY", "CAD/CHF", "CHF/JPY",
  "USD/SEK", "USD/NOK", "USD/DKK", "USD/PLN", "USD/HUF", "USD/CZK",
  "USD/TRY", "USD/ZAR", "USD/MXN", "USD/SGD", "USD/HKD", "USD/CNH",
  "EUR/SEK", "EUR/NOK", "EUR/PLN", "EUR/TRY", "EUR/HUF", "EUR/CZK", "EUR/ZAR",
  "GBP/SEK", "GBP/NOK",
].map((symbol) => ({ symbol, name: symbol }));

export const RESEARCH_INSTRUMENTS = [
  ...FX_PAIRS,
  { symbol: "XAU/USD", name: "Gold" },
  { symbol: "US30", name: "Dow Jones" },
];

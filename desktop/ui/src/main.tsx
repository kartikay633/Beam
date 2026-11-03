import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri } from "./api";
import { App } from "./App";
import { OfferWindow } from "./Offer";
import "./styles.css";

// In the app the window has Windows 11's Mica backdrop, so the page lets it
// show through; in a browser preview the page paints its own background.
if (isTauri) document.documentElement.classList.add("mica");

// The incoming-offer popup is a separate window that loads #/offer/<id>.
const offer = window.location.hash.match(/^#\/offer\/(.+)$/);

createRoot(document.getElementById("root")!).render(
  <StrictMode>{offer ? <OfferWindow id={offer[1]} /> : <App />}</StrictMode>,
);

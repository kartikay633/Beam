import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { OfferWindow } from "./Offer";
import "./styles.css";

// The incoming-offer popup is a separate window that loads #/offer/<id>.
const offer = window.location.hash.match(/^#\/offer\/(.+)$/);

createRoot(document.getElementById("root")!).render(
  <StrictMode>{offer ? <OfferWindow id={offer[1]} /> : <App />}</StrictMode>,
);

import type { StoryWorkerClient } from "./story-client.js";
export class WellfriendFontPicker extends HTMLElement {
  client:StoryWorkerClient;
  disabled:boolean;
}
declare global { interface HTMLElementTagNameMap { "wellfriend-font-picker":WellfriendFontPicker } }

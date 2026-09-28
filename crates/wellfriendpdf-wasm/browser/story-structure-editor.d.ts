import type {StoryRequest, StoryWorkerClient} from "./story-client.js";
export class WellfriendStoryStructureEditor extends HTMLElement {
  client:StoryWorkerClient|undefined;
  begin(base:StoryRequest):Promise<void>;
}
declare global {interface HTMLElementTagNameMap {"wellfriend-story-structure-editor":WellfriendStoryStructureEditor}}

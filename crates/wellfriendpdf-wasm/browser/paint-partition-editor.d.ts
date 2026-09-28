import type {PaintPartitionRequest,StoryWorkerClient} from "./story-client.js";
export class WellfriendPaintPartitionEditor extends HTMLElement {
  client:StoryWorkerClient|undefined;
  request:PaintPartitionRequest|undefined;
}
declare global {interface HTMLElementTagNameMap {"wellfriend-paint-partition-editor":WellfriendPaintPartitionEditor}}

import assert from 'node:assert/strict';
import type {ApiDocument,DeploymentClient} from '../src/api.ts';
export type Call = Parameters<DeploymentClient['request']>;
export function payload(call: Call): ApiDocument {
 const value=call[2];assert(value&&typeof value==='object'&&!Array.isArray(value));return value;
}
export function toolText(result: unknown): string {
 assert(result&&typeof result==='object'&&'content' in result&&Array.isArray(result.content));
 const item=result.content[0];assert(item&&typeof item==='object'&&item.type==='text'&&typeof item.text==='string');return item.text;
}

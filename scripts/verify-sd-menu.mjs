// Exercise the real Vue templates with a test renderer and simulated Tauri responses.
// No physical disks are accessed.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse, compileScript } from '@vue/compiler-sfc';
import ts from 'typescript';
import * as Vue from 'vue';

const timers = new Map();
let nextTimer = 0;
const originalInterval = globalThis.setInterval;
const originalClear = globalThis.clearInterval;
globalThis.setInterval = callback => { timers.set(++nextTimer, callback); return nextTimer; };
globalThis.clearInterval = id => timers.delete(id);
globalThis.window = new EventTarget();
const storage = new Map([['sddirecta-selected-drive', 'H:']]);
globalThis.localStorage = { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) };

let driveResult = [
	{ drive: 'C:', type: 'Fixed', size: 1024 ** 4 },
	{ drive: 'E:', type: 'Removable', size: -1 },
	{ drive: 'H:', type: 'Removable', size: 8 * 1024 ** 3 },
];
let matchedResult = { files: ['FILE-AA.001'], from_input: false };
let failDrives = false;
let failMatches = false;
const calls = [];
async function invoke(command) {
	calls.push(command);
	if (command === 'get_drives') {
		if (failDrives) throw new Error('drive test failure');
		return driveResult;
	}
	if (command === 'find_dictionary_files') {
		if (failMatches) throw new Error('match test failure');
		return matchedResult;
	}
	if (command === 'get_executable_directory') return 'C:\\sd-test';
	if (command === 'check_startup_csv') return true;
	if (command === 'read_startup_csv') return 'A,FILE-AA.001,00001000,00001FFF';
	throw new Error(`Unexpected command: ${command}`);
}

function loadTs(path, name) {
	const source = readFileSync(new URL(path, import.meta.url), 'utf8');
	const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText;
	const exports = {};
	new Function('exports', js)(exports);
	return exports[name];
}
const splitFolderPath = loadTs('../src/utils/executablePath.ts', 'splitFolderPath');
const parseSdDirectCsv = loadTs('../src/utils/sdDirectCsv.ts', 'parseSdDirectCsv');
function loadComponent(path, SdMenu) {
	const source = readFileSync(new URL(path, import.meta.url), 'utf8');
	const { descriptor } = parse(source);
	let code = compileScript(descriptor, { id: path, inlineTemplate: true }).content;
	code = code.replace(/import\s+\{([^}]+)\}\s+from\s+['"]vue['"];?/g,
		(_, names) => `const {${names.replace(/\s+as\s+/g, ': ')}} = Vue;`);
	code = code.replace(/^import .*?;\s*$/gm, '').replace('export default', 'return');
	code = ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext } }).outputText;
	return new Function('Vue', 'invoke', 'SdMenu', 'splitFolderPath', 'parseSdDirectCsv', code)(Vue, invoke, SdMenu, splitFolderPath, parseSdDirectCsv);
}

function node(type, text = '') { return Object.assign(new EventTarget(), { type, text, props: {}, children: [], parent: null }); }
const renderer = Vue.createRenderer({
	createElement: type => node(type), createText: text => node('#text', text), createComment: text => node('#comment', text),
	setText: (target, text) => { target.text = text; },
	setElementText: (target, text) => { target.text = text; target.children = []; },
	patchProp: (target, key, _prev, value) => {
		target.props[key] = value;
		if (key === 'value') target.value = value;
	},
	insert: (target, parent, anchor) => {
		if (target.parent) target.parent.children.splice(target.parent.children.indexOf(target), 1);
		target.parent = parent;
		const index = anchor ? parent.children.indexOf(anchor) : -1;
		parent.children.splice(index < 0 ? parent.children.length : index, 0, target);
	},
	remove: target => { if (target.parent) target.parent.children.splice(target.parent.children.indexOf(target), 1); },
	parentNode: target => target.parent,
	nextSibling: target => target.parent?.children[target.parent.children.indexOf(target) + 1],
});
const root = node('root');
function all(target = root) { return [target, ...target.children.flatMap(child => all(child))]; }
function byClass(name) { return all().filter(target => target.props.class?.split(' ').includes(name)); }
const button = () => byClass('sd-write-button')[0];
const settle = async () => { for (let i = 0; i < 20; i++) { await Promise.resolve(); await Vue.nextTick(); } };
const poll = async () => { for (const callback of [...timers.values()]) callback(); await settle(); };
const chooseDrive = async drive => {
	all().find(target => target.type === 'input' && target.props.value === drive).dispatchEvent(new Event('change'));
	await settle();
};

try {
	const SdMenu = loadComponent('../src/components/SdMenu.vue');
	const Home = loadComponent('../src/pages/HomeView.vue', SdMenu);
	const app = renderer.createApp(Home);
	app.mount(root);
	assert.equal(button().props.disabled, true, 'disabled before files are matched');
	await settle();
	assert.equal(button().props.disabled, false, 'matching files and media in the selected drive enable writing');
	assert.equal(byClass('drive-item-inline').length, 2, 'fixed disks are hidden');
	assert.equal(byClass('status-empty').length, 1);
	assert.equal(byClass('status-inserted').length, 1);
	assert.equal(byClass('sd-capacity-display-inline')[0].text, '8GB');
	assert.equal(all().find(target => target.type === 'input' && target.props.value === 'H:').checked, true, 'restores the selected drive');
	await chooseDrive('E:');
	assert.equal(button().props.disabled, true, 'selecting an empty drive disables writing even when another drive has media');
	await chooseDrive('H:');
	assert.equal(button().props.disabled, false, 'selecting a drive with media enables writing');

	driveResult = driveResult.map(drive => ({ ...drive, size: -1 }));
	await poll();
	assert.equal(byClass('status-empty').length, 2, 'removing media turns the icon gray');
	assert.equal(byClass('sd-capacity-display-inline').length, 0, 'removing media hides capacity');
	assert.equal(button().props.disabled, true, 'removing selected media disables writing even with matching files');
	const messageCount = byClass('path-message').length;
	button().props.onClick();
	await settle();
	assert.equal(byClass('path-message').length, messageCount, 'disabled write handler does not emit a write request');

	driveResult = driveResult.map(drive => drive.drive === 'E:' ? { ...drive, size: 4 * 1024 ** 3 } : drive);
	await poll();
	assert.equal(byClass('status-inserted').length, 1, 'inserting media turns the icon blue');
	assert.equal(byClass('sd-capacity-display-inline')[0].text, '4GB');
	assert.equal(button().props.disabled, true, 'media in an unselected drive does not enable writing');
	await chooseDrive('E:');
	assert.equal(button().props.disabled, false, 'choosing the inserted card enables writing');
	matchedResult = { files: [], from_input: false };
	await poll();
	assert.equal(button().props.disabled, true, 'removing all matching files disables writing');
	matchedResult = { files: ['unrelated.txt'], from_input: false };
	await poll();
	assert.equal(button().props.disabled, true, 'unrelated files do not enable writing');
	matchedResult = { files: ['FILE-AA.001'], from_input: true };
	await poll();
	assert.equal(button().props.disabled, false, 'input-folder matches enable writing');
	assert.ok(all().some(target => target.props.title === 'input'), 'input folder is displayed');
	failMatches = true;
	await poll();
	assert.equal(button().props.disabled, true, 'matching failures clear stale enablement');
	failMatches = false;
	failDrives = true;
	await poll();
	assert.equal(byClass('backend-indicator--stopped').length, 1);
	assert.equal(byClass('drive-item-inline').length, 0, 'drive errors clear stale drive status');
	assert.equal(button().props.disabled, true, 'drive errors disable writing even with matching files');
	failDrives = false;
	await poll();
	assert.equal(byClass('backend-indicator--running').length, 1, 'backend recovers');
	const event = new Event('keydown', { cancelable: true });
	Object.defineProperty(event, 'key', { value: 'F5' });
	const previousCalls = calls.length;
	window.dispatchEvent(event);
	await settle();
	assert.equal(event.defaultPrevented, true);
	assert.ok(calls.length > previousCalls, 'F5 refreshes drive state');
	app.unmount();
	assert.equal(timers.size, 0, 'polling stops when unmounted');
	console.log('PASS: files AND selected media enablement, drive switching, removal guard, capacity, error recovery, F5 and cleanup');
} finally {
	globalThis.setInterval = originalInterval;
	globalThis.clearInterval = originalClear;
}

#!/usr/bin/env node
// Start a task in its own worktree and herdr tab with a fresh Claude session, as CLAUDE.md
// describes ("Parallel work: one worktree per task").
// Usage: node task-start.mjs <task>[:model[:effort]] --brief <file> [--base master]
//        node task-start.mjs --close <task>...   (close the tab, remove a clean worktree;
//                                                  the branch goes only when merged into master)
//        node task-start.mjs --list
import { execFileSync } from 'node:child_process';
import { appendFileSync, copyFileSync, existsSync, rmSync, statfsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

const sh = (cmd, a, opts = {}) => execFileSync(cmd, a, { encoding: 'utf8', ...opts }).trim();
const herdr = (...a) => JSON.parse(sh('herdr', a)).result;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The main checkout, also when this copy of the script runs from a worktree.
const REPO = dirname(resolve(import.meta.dirname, sh('git', ['-C', import.meta.dirname, 'rev-parse', '--git-common-dir'])));
const worktree = (task) => `${REPO}/.claude/worktrees/${task}`;
const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];
// Each worktree's target/ starts as a reflink copy, but every build in it takes new space.
const MIN_FREE_GB = 20;

const args = process.argv.slice(2);
const opt = (name) => {
	const i = args.indexOf(name);
	return i >= 0 ? args.splice(i, 2)[1] : undefined;
};
const flag = (name) => {
	const i = args.indexOf(name);
	return i >= 0 && args.splice(i, 1).length > 0;
};

function usage(msg) {
	if (msg) console.error(`task-start: ${msg}`);
	console.error('usage: task-start.mjs <task>[:model[:effort]] --brief <file> [--base master] | --close <task>... | --list');
	process.exit(2);
}

if (process.env.HERDR_ENV !== '1') usage('not running inside herdr (HERDR_ENV != 1)');
const WORKSPACE = process.env.HERDR_WORKSPACE_ID;
const tabs = () => herdr('tab', 'list', '--workspace', WORKSPACE).tabs;
const findTab = (label) => tabs().find((t) => t.label === label);

function list() {
	const wts = sh('git', ['-C', REPO, 'worktree', 'list', '--porcelain']).split('\n\n')
		.map((b) => Object.fromEntries(b.split('\n').map((l) => [l.split(' ')[0], l.slice(l.indexOf(' ') + 1)])))
		.filter((w) => w.worktree.startsWith(`${REPO}/.claude/worktrees/`));
	if (!wts.length) return console.log('no task worktrees');
	for (const w of wts) {
		const task = w.worktree.split('/').pop();
		const tab = findTab(task);
		const ahead = sh('git', ['-C', REPO, 'rev-list', '--count', `master..${w.HEAD}`]);
		const dirty = sh('git', ['-C', w.worktree, 'status', '--porcelain']) ? ', uncommitted changes' : '';
		console.log(`${task}: ${w.branch?.replace('refs/heads/', '') ?? 'detached'}, ${ahead} commits ahead of master${dirty}, ` +
			(tab ? `tab ${tab.tab_id} (${tab.agent_status})` : 'no tab'));
	}
}

async function start(spec, briefFile, base) {
	const [task, model, effort] = spec.split(':');
	if (!/^[a-z0-9][a-z0-9-]*$/.test(task)) usage(`task name "${task}": lowercase letters, digits and dashes`);
	if (effort && !EFFORTS.includes(effort)) usage(`unknown effort "${effort}" (${EFFORTS.join('|')})`);
	if (!briefFile || !existsSync(briefFile)) usage('--brief <file> is required');
	if (findTab(task)) throw new Error(`herdr tab ${task} already exists`);

	const wt = worktree(task);
	const branch = `feature/${task}`;
	if (!existsSync(wt)) {
		const fs = statfsSync(REPO);
		const freeGb = (fs.bavail * fs.bsize) / 1e9;
		if (freeGb < MIN_FREE_GB) throw new Error(`only ${freeGb.toFixed(0)} GB free - close finished tasks (--close) first`);
		const exists = sh('git', ['-C', REPO, 'branch', '--list', branch]);
		sh('git', ['-C', REPO, 'worktree', 'add', '-q', ...(exists ? [wt, branch] : ['-b', branch, wt, base])]);
		// target/ of the main checkout as a reflink copy: gpui-kit is already built, and btrfs
		// shares the blocks. A copy cut short is worse than none (cargo trusts what it finds).
		process.stdout.write(`${task}: copying target/… `);
		try {
			sh('cp', ['-a', '--reflink=always', `${REPO}/target`, `${wt}/target`]);
		} catch (e) {
			rmSync(`${wt}/target`, { recursive: true, force: true });
			throw new Error(`reflink copy of target/ failed (worktree kept without it): ${e.stderr || e.message}`);
		}
		console.log('ok');
	}

	// The brief lives in the worktree's git dir: outside the working tree, removed with it.
	const gitDir = resolve(wt, sh('git', ['-C', wt, 'rev-parse', '--git-dir']));
	const brief = `${gitDir}/task-brief.md`;
	copyFileSync(briefFile, brief);
	appendFileSync(brief, `
## Always
- You work in the worktree \`${wt}\` on branch \`${branch}\`. Its \`target/\` is a reflink copy of the main checkout's.
- Read \`CLAUDE.md\` first. Project memory may not load in a worktree: read
  \`/home/cechi/.claude/projects/-home-cechi-dev-vais-apps/memory/MEMORY.md\` and the files it lists that matter here.
- Commit only to \`${branch}\` (\`cargo fmt --all\` first, small commits). No push, no commits to master, no
  \`scripts/install-desktop.sh\`. The main session merges.
- UI checks in your own nested niri, as CLAUDE.md describes.
- The user talks to you in this herdr tab and writes in Czech; answer in Czech. Ask them where a decision is theirs.
- At the end write a short summary in Czech: what you did, the commits, open questions.
`);

	const tab = herdr('tab', 'create', '--workspace', WORKSPACE, '--cwd', wt, '--label', task, '--no-focus');
	const pane = tab.root_pane.pane_id;
	herdr('agent', 'start', task, '--kind', 'claude', '--pane', pane, '--timeout', '90000', '--',
		'--permission-mode', 'auto', '-n', task, ...(model ? ['--model', model] : []), ...(effort ? ['--effort', effort] : []));
	herdr('agent', 'prompt', task, `Přečti si zadání v ${brief} a pusť se do něj.`);

	// The prompt can fail at once (e.g. a model without usage credits left): report it.
	await sleep(5000);
	const screen = sh('herdr', ['agent', 'read', task]);
	const failure = screen.match(/out of usage credits|Invalid model|API Error[^\n]*/i);
	if (failure) {
		throw new Error(`agent stopped right away: "${failure[0]}" - tab ${tab.tab.tab_id} left open; fix it there (e.g. /model opus) and resend the prompt`);
	}
	console.log(`${task}: started in tab ${tab.tab.tab_id} (pane ${pane}), branch ${branch}` +
		`${model || effort ? `, ${model ?? 'default'}/${effort ?? 'default'}` : ''}, brief ${brief}`);
}

function close(task) {
	const wt = worktree(task);
	const tab = findTab(task);
	if (tab) herdr('tab', 'close', tab.tab_id);
	if (existsSync(wt)) {
		if (sh('git', ['-C', wt, 'status', '--porcelain'])) throw new Error(`${wt} has uncommitted changes - tab closed, worktree kept`);
		// --force: the ignored target/ is the only thing left in it
		sh('git', ['-C', REPO, 'worktree', 'remove', '--force', wt]);
	}
	const branch = `feature/${task}`;
	let branchNote = 'no branch';
	if (sh('git', ['-C', REPO, 'branch', '--list', branch])) {
		const merged = sh('git', ['-C', REPO, 'branch', '--merged', 'master', '--list', branch]);
		if (merged) {
			sh('git', ['-C', REPO, 'branch', '-d', branch]);
			branchNote = `branch ${branch} deleted (merged)`;
		} else branchNote = `branch ${branch} kept (not merged into master)`;
	}
	console.log(`${task}: ${tab ? 'tab closed, ' : ''}worktree removed, ${branchNote}`);
}

let failed = 0;
if (flag('--list')) {
	list();
} else if (flag('--close')) {
	if (!args.length) usage('--close needs a task name');
	for (const task of args) {
		try {
			close(task);
		} catch (e) {
			failed++;
			console.error(`${task}: ${e.message}`);
		}
	}
} else {
	const brief = opt('--brief');
	const base = opt('--base') ?? 'master';
	if (args.length !== 1) usage('one task per start');
	try {
		await start(args[0], brief, base);
	} catch (e) {
		failed++;
		console.error(`${args[0].split(':')[0]}: ${e.message}`);
	}
}
process.exit(failed ? 1 : 0);

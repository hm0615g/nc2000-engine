'use strict';

const {spawn} = require('node:child_process');
const readline = require('node:readline');

class NativeProtocolWorker {
	constructor(options) {
		this.repo = options.repo;
		this.binary = options.binary;
		this.child = null;
		this.stdoutLines = null;
		this.stderrLines = null;
		this.seq = 0;
		this.pending = new Map();
		this.closed = false;
		this.exitPromise = null;
	}

	start() {
		if (this.child) return;

		if (this.closed) {
			throw new Error(
				'native protocol worker has already been closed'
			);
		}

		const child = spawn(
			this.binary,
			[],
			{
				cwd: this.repo,
				stdio: ['pipe', 'pipe', 'pipe'],
			}
		);

		this.child = child;

		this.stdoutLines = readline.createInterface({
			input: child.stdout,
		});

		this.stderrLines = readline.createInterface({
			input: child.stderr,
		});

		this.stdoutLines.on('line', line => {
			let msg;

			try {
				msg = JSON.parse(line);
			} catch {
				this.failAll(
					new Error(
						`native worker wrote non-JSON stdout: ${line}`
					)
				);
				return;
			}

			const waiter = this.pending.get(msg.seq);

			if (!waiter) {
				this.failAll(
					new Error(
						`native worker returned unexpected seq ${msg.seq}`
					)
				);
				return;
			}

			this.pending.delete(msg.seq);

			if (!msg.ok) {
				waiter.reject(
					new Error(
						`native worker: ${msg.error || 'unknown error'}`
					)
				);
				return;
			}

			waiter.resolve(msg);
		});

		this.stderrLines.on('line', line => {
			console.error(`[native-search] ${line}`);
		});

		this.exitPromise = new Promise(resolve => {
			child.once('exit', (code, signal) => {
				const expected = this.closed;

				this.child = null;

				if (!expected) {
					this.closed = true;

					this.failAll(
						new Error(
							`native worker exited unexpectedly ` +
							`(code=${code}, signal=${signal || 'none'})`
						)
					);
				}

				resolve({code, signal});
			});
		});

		child.once('error', error => {
			this.failAll(error);
		});
	}

	failAll(error) {
		for (const waiter of this.pending.values()) {
			waiter.reject(error);
		}
		this.pending.clear();
	}

	rpc(command) {
		this.start();

		const id = ++this.seq;

		return new Promise((resolve, reject) => {
			this.pending.set(id, {resolve, reject});

			const text =
				JSON.stringify({
					seq: id,
					...command,
				}) + '\n';

			this.child.stdin.write(text, error => {
				if (!error) return;

				const waiter = this.pending.get(id);

				if (waiter) {
					this.pending.delete(id);
					waiter.reject(error);
				}
			});
		});
	}

	async hello() {
		return this.rpc({
			cmd: 'hello',
		});
	}

	async shutdown() {
		if (this.closed) {
			return this.exitPromise;
		}

		this.start();
		this.closed = true;

		try {
			await this.rpc({
				cmd: 'shutdown',
			});
		} finally {
			if (this.child) {
				this.child.stdin.end();
			}
		}

		return this.exitPromise;
	}

	kill() {
		this.closed = true;

		if (this.child) {
			this.child.kill('SIGKILL');
		}

		this.failAll(
			new Error('native worker killed')
		);
	}
}

class NativeProtocolSession {
	constructor(worker, options) {
		this.worker = worker;
		this.battle = options.battle;
		this.options = options;

		this.initialized = false;
		this.closed = false;

		this.iterationCount = 0;
		this.baked = null;
		this.previewInfo = null;
		this.root = null;
		this.belief = null;
		this.state = null;
		this.metric = {
			legalityDrift: 0,
			projections: 0,
		};
	}

	async init() {
		if (this.initialized) return;

		const payload = {
			cmd: 'init',
			battle: this.battle,
			side: this.options.side,
			seed: this.options.seed,
			c: this.options.c,
			mode: this.options.mode,
			ownTeam: this.options.ownTeam,
		};

		if (this.options.opponentTeam) {
			payload.opponentTeam =
				this.options.opponentTeam;
		}

		if (this.options.beliefPrior) {
			payload.beliefPrior =
				this.options.beliefPrior;
		}

		const result =
			await this.worker.rpc(payload);

		this.initialized = true;

		return result;
	}

	async pushLines(lines) {
		await this.init();

		if (typeof lines === 'string') {
			lines = JSON.parse(lines);
		}

		if (!lines || !lines.length) return;

		await this.worker.rpc({
			cmd: 'feed',
			battle: this.battle,
			lines,
		});
	}

	async onRequest(request) {
		await this.init();

		const result =
			await this.worker.rpc({
				cmd: 'request',
				battle: this.battle,
				request,
			});

		this.iterationCount =
			result.iterations || 0;

		this.baked =
			result.bakedPreview ?? null;

		this.previewInfo =
			result.previewInfo ?? null;

		if (result.owes) {
			await this.inspect();
		} else {
			this.root = null;
			this.belief = null;
			this.state = null;
		}

		return result.owes;
	}

	async step(n) {
		const result =
			await this.worker.rpc({
				cmd: 'step',
				battle: this.battle,
				n,
			});

		this.iterationCount =
			result.iterations;

		return this.iterationCount;
	}

	async best() {
		const result =
			await this.worker.rpc({
				cmd: 'best',
				battle: this.battle,
			});

		this.iterationCount =
			result.iterations;

		await this.inspect();

		return result.choice ?? null;
	}

	async inspect() {
		const result =
			await this.worker.rpc({
				cmd: 'inspect',
				battle: this.battle,
			});

		this.iterationCount =
			result.iterations;

		this.baked =
			result.bakedPreview ?? null;

		this.previewInfo =
			result.previewInfo ?? null;

		this.root =
			result.rootPolicy ?? null;

		this.belief =
			result.beliefInfo ?? null;

		this.state =
			result.stateView ?? null;

		this.metric =
			result.metrics || this.metric;

		return result;
	}

	async cancel() {
		if (
			!this.initialized ||
			this.closed
		) {
			return;
		}

		await this.worker.rpc({
			cmd: 'cancel',
			battle: this.battle,
		});
	}

	async close() {
		if (
			!this.initialized ||
			this.closed
		) {
			this.closed = true;
			return;
		}

		try {
			await this.worker.rpc({
				cmd: 'close',
				battle: this.battle,
			});
		} finally {
			this.closed = true;
		}
	}

	free() {
		void this.close().catch(error => {
			console.error(
				`[native-search] session close failed: ${error.message || error}`
			);
		});
	}

	iterations() {
		return this.iterationCount;
	}

	bakedPreview() {
		return this.baked;
	}

	previewCacheInfo() {
		return this.previewInfo ?
			JSON.stringify(this.previewInfo) :
			undefined;
	}

	rootPolicy() {
		return JSON.stringify(
			this.root || {
				iterations: this.iterationCount,
				baked: false,
				forced: null,
				actions: [],
			}
		);
	}

	beliefInfo() {
		return JSON.stringify(
			this.belief || {
				mode: this.options.mode,
				candidateCount: 0,
				isFallback: false,
				candidateIds: [],
				sampledCandidateId: null,
				sampledCandidateName: null,
				fallbackReason: null,
				lastObserverRevision: null,
				count: 0,
				fallback: false,
				candidates: [],
			}
		);
	}

	stateView() {
		if (!this.state) {
			throw new Error(
				'native stateView unavailable before onRequest'
			);
		}

		return JSON.stringify(this.state);
	}

	metrics() {
		return JSON.stringify(this.metric);
	}
}

module.exports = {
	NativeProtocolWorker,
	NativeProtocolSession,
};

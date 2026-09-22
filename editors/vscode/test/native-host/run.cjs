const { run } = require('./runner.cjs');
const { FORMATS } = require('./contract.cjs');

run(process.env, process.argv.slice(2)).then((result) => {
	console.log(JSON.stringify(result, null, 2));
	process.exitCode = result.exitCode;
}).catch((error) => {
	console.error(JSON.stringify({ format: FORMATS.error, status: 'failed', code: error.code ?? 'E_NATIVE_RUN', message: error.message }));
	process.exitCode = error.exitCode ?? 1;
});

export interface AddressRange {
	startAddress: string;
	endAddress: string;
}

/** Addresses remain strings to preserve hexadecimal notation and leading zeros. */
export function parseSdDirectCsv(csv: string): {
	dictionary: Map<string, AddressRange>;
	addressRange: AddressRange | null;
} {
	const dictionary = new Map<string, AddressRange>();
	let minimumStart: bigint | undefined;
	let maximumEnd: bigint | undefined;
	for (const [index, line] of csv.split(/\r\n|\n|\r/).entries()) {
		const fields = line.split(',').map(field => field.trim());
		if (fields[0] !== 'A') continue;
		const [, filename, startAddress, endAddress] = fields;
		if (!filename || !startAddress || !endAddress) {
			throw new Error(`${index + 1}行目: ファイル名・開始アドレス・終了アドレスが必要です`);
		}
		if (![startAddress, endAddress].every(address => /^(?:0x)?[0-9a-f]+$/i.test(address))) {
			throw new Error(`${index + 1}行目: 開始アドレス・終了アドレスは16進数で指定してください`);
		}
		const start = BigInt(`0x${startAddress.replace(/^0x/i, '')}`);
		const end = BigInt(`0x${endAddress.replace(/^0x/i, '')}`);
		if (minimumStart === undefined || start < minimumStart) minimumStart = start;
		if (maximumEnd === undefined || end > maximumEnd) maximumEnd = end;
		dictionary.set(filename, { startAddress, endAddress });
	}
	const formatAddress = (address: bigint) => `0x${address.toString(16).toUpperCase().padStart(8, '0')}`;
	return {
		dictionary,
		addressRange: minimumStart === undefined || maximumEnd === undefined ? null : {
			startAddress: formatAddress(minimumStart),
			endAddress: formatAddress(maximumEnd),
		},
	};
}

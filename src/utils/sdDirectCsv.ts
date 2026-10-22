export interface AddressRange {
	startAddress: string;
	endAddress: string;
}

export interface SdArea extends AddressRange {
	accessFlag: string;
}

/** Addresses remain strings to preserve hexadecimal notation and leading zeros. */
export function parseSdDirectCsv(csv: string): {
	dictionary: Map<string, SdArea>;
	addressRange: AddressRange | null;
} {
	const dictionary = new Map<string, SdArea>();
	let minimumStart: bigint | undefined;
	let maximumEnd: bigint | undefined;
	for (const [index, line] of csv.split(/\r\n|\n|\r/).entries()) {
		if (!line.trim()) continue;
		const fields = line.split(',').map(field => field.trim());
		const [accessFlag, filename, startAddress, endAddress] = fields;
		if (accessFlag === 'E') break;
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
		// Keep every area's flag; each SD operation selects its own targets later.
		dictionary.set(filename, { accessFlag, startAddress, endAddress });
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

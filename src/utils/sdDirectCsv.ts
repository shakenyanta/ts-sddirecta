export interface AddressRange {
	startAddress: string;
	endAddress: string;
}

/** Addresses remain strings to preserve hexadecimal notation and leading zeros. */
export function parseSdDirectCsv(csv: string): Map<string, AddressRange> {
	const dictionary = new Map<string, AddressRange>();
	for (const [index, line] of csv.split(/\r\n|\n|\r/).entries()) {
		const fields = line.split(',').map(field => field.trim());
		if (fields[0] !== 'A') continue;
		const [, filename, startAddress, endAddress] = fields;
		if (!filename || !startAddress || !endAddress) {
			throw new Error(`${index + 1}行目: ファイル名・開始アドレス・終了アドレスが必要です`);
		}
		dictionary.set(filename, { startAddress, endAddress });
	}
	return dictionary;
}

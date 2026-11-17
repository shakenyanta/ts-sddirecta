export interface DailyReportField { name: string; size: number; format?: 'datetime' | 'payment' }
export interface DailyReportRecord { offset: number; length: number; stopCount: number }
export interface DailyReportResult {
    records: DailyReportRecord[];
    warnings: string[];
    checksum: string | null;
}
export interface DailyReportRow extends DailyReportField { content: string; detail: string }

// 「スタートアドレス」「文字数」は解析情報であり、ファイルから消費する項目には含めない。
export const dailyReportFields: readonly DailyReportField[] = [
    { name: 'ヘッダ', size: 1 },
    { name: '乗車:経度', size: 4 },
    { name: '乗車:緯度', size: 3 },
    { name: '乗車:日時(日月年時分)', size: 5, format: 'datetime' },
    { name: '降車:経度', size: 4 },
    { name: '降車:緯度', size: 3 },
    { name: '降車:日時(日月年時分)', size: 5, format: 'datetime' },
    { name: '人数', size: 1 },
    { name: '料金', size: 4 },
    { name: 'メータ外料金', size: 3 },
    { name: '現金', size: 3 },
    { name: '■支払区分', size: 1, format: 'payment' },
    { name: 'その他1', size: 3 },
    { name: 'その他2', size: 3 },
    { name: 'メーター料金', size: 4 },
    { name: '営業回数', size: 2 },
    { name: '事前確定金額,DeNA貸切,DeNA定額(立替,ETC料金は除く)', size: 4 },
    { name: 'タブレット障害者割引金額(事前確定)', size: 4 },
    { name: 'ETC料金', size: 3 },
    { name: '割引額(障割,高割,幼割,他割)', size: 3 },
    { name: '遠距離割引額', size: 3 },
    { name: '固定料金(迎車,早朝,ワゴン,予約)', size: 3 },
    { name: 'チケット金額', size: 3 },
    { name: 'ネット決済種別名称', size: 14 },
    { name: 'ネット決済結果', size: 1 },
    { name: 'DeNA割引額', size: 3 },
    { name: 'モバクリ', size: 3 },
    { name: 'その他1', size: 3 },
    { name: 'その他2', size: 3 },
    { name: 'その他3', size: 3 },
    { name: 'アプリ手配料金', size: 3 },
    { name: '跳ね上がり回数', size: 1 },
    { name: '割引カード番号(8桁)桁数に満たない場合は頭スペース', size: 4 },
    { name: '割引カード割引額(円)', size: 3 },
    { name: '未収種別', size: 1 },
    { name: '領収書印字枚数', size: 1 },
    { name: '測地系', size: 1 },
    { name: '運賃種別', size: 1 },
    { name: '予備', size: 84 },
];
export const dailyReportBaseLength = dailyReportFields.reduce((sum, field) => sum + field.size, 0);
export const dailyReportStopLength = 12;

export const paymentTypeDictionary: Readonly<Partial<Record<string, string>>> = {
    '00': 'カード決済',
    '01': '現収',
    '02': '未収／チケット',
    '03': 'カード現金混在',
    '04': 'デビット現金混在',
    '05': 'カードチケット混在',
    '06': '現金チケットカード混在',
    '07': '燃料',
    '08': '休憩（自動休憩）',
    '09': '合計押無し',
    '0A': '迎車キャンセル',
    '0B': '休憩（手動）',
    '11': '回送（メータ操作）',
    '19': '置換（メータ操作）',
    '20': '出庫',
    '21': '入庫',
    '27': '待機（メータ操作）',
    '28': '貸切（メータ操作）',
    '4F': 'プリカ決済',
    '41': 'プリカ現金混在',
    '42': 'プリカチケット混在',
    '44': 'プリカチケット現金混在',
    '5F': 'Edyのみ',
    '51': 'Edy現金混在',
    '52': 'Edyチケット混在',
    '54': 'Edyチケット現金混在',
    '6F': '交通系IC（スイカ・パスモ）決済',
    '61': '交通系IC（スイカ・パスモ）現金混在',
    '62': '交通系IC（スイカ・パスモ）チケット混在',
    '64': '交通系IC（スイカ・パスモ）チケット現金混在',
    '7F': 'QP決済',
    '71': 'QP現金混在',
    '72': 'QPチケット混在',
    '74': 'QPチケット現金混在',
    '8F': 'iD決済',
    '81': 'iD現金混在',
    '82': 'iDチケット混在',
    '84': 'iDチケット現金混在',
    '9F': 'タクパ決済',
    'AF': '銀聯',
    'A2': '銀聯チケット混在',
    'C0': 'WAON決済',
    'C1': 'WAON現金混在',
    'C2': 'WAONチケット混在',
    'C4': 'WAONチケット現金混在',
    'DF': 'nanaco決済',
    'D1': 'nanaco＋現金',
    'D2': 'nanaco＋チケット',
    'D4': 'nanaco＋現金・チケット',
    'EF': 'ネット決済',
    'E1': 'ネット決済＋現金',
    'E2': 'ネット決済＋チケット',
    'E4': 'ネット決済＋現金＋チケット',
};

function hex(bytes: Uint8Array): string {
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0').toUpperCase()).join('');
}

function formatDateTime(content: string): string {
    // 日・月・年（下2桁）・時・分のBCD。西暦は2000～2099年として表示する。
    if (!/^\d{10}$/.test(content)) return '日時不正';
    const [day, month, shortYear, hour, minute] = content.match(/\d{2}/g)!.map(Number);
    const year = 2000 + shortYear;
    const date = new Date(Date.UTC(year, month - 1, day));
    if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1 || date.getUTCDate() !== day
        || hour > 23 || minute > 59) return '日時不正';
    return `${year}年${content.slice(2, 4)}月${content.slice(0, 2)}日 ${content.slice(6, 8)}時${content.slice(8, 10)}分`;
}

function formatDetail(field: DailyReportField, content: string): string {
    switch (field.format) {
        case 'datetime': return formatDateTime(content);
        case 'payment': return paymentTypeDictionary[content] ?? '未定義';
        default: return '';
    }
}

export function parseDailyReport(bytes: Uint8Array): DailyReportResult {
    if (bytes.length < 3) throw new Error('先頭3バイトを読み取れません。日報ファイルが短すぎます。');
    let offset = bytes.indexOf(0xce, 3);
    if (offset < 0) throw new Error('先頭3バイト以降に日報ヘッダCEがありません。');
    const result: DailyReportResult = { records: [], warnings: [], checksum: null };
    if (offset !== 3) result.warnings.push(`先頭3バイトの直後にCEがなく、0x${offset.toString(16).toUpperCase()}から解析しています。`);

    // outputはSD領域全体のため、末尾にFFの未使用領域が付く場合がある。
    // FC E6等の固定値を検索せず、候補レコードの直後2バイトだけをチェックサム候補とする。
    let usedEnd = bytes.length;
    while (usedEnd > 3 && bytes[usedEnd - 1] === 0xff) usedEnd--;

    while (offset < bytes.length) {
        const candidates: { length: number; stopCount: number; terminal: boolean; checksum: boolean }[] = [];
        for (let stopCount = 0; stopCount <= 3; stopCount++) {
            const length = dailyReportBaseLength + dailyReportStopLength * stopCount;
            const end = offset + length;
            if (end > bytes.length) continue;
            if (end === bytes.length) {
                candidates.push({ length, stopCount, terminal: true, checksum: false });
            } else if (end + 2 === bytes.length
                || (end + 2 <= bytes.length && usedEnd >= end && usedEnd <= end + 2)) {
                candidates.push({ length, stopCount, terminal: true, checksum: true });
            } else if (bytes[end] === 0xce) {
                candidates.push({ length, stopCount, terminal: false, checksum: false });
            }
        }
        if (candidates.length !== 1) {
            const position = `0x${offset.toString(16).toUpperCase()}`;
            result.warnings.push(candidates.length > 1
                ? `${position}: 次のCEまたは終端の候補が複数あるため、この営業以降の解析を停止しました。`
                : `${position}: 次のCEまたは終端を確定できません（長さ不足・形式不一致の可能性）。この営業以降の解析を停止しました。`);
            break;
        }
        const candidate = candidates[0];
        result.records.push({ offset, length: candidate.length, stopCount: candidate.stopCount });
        offset += candidate.length;
        if (candidate.terminal) {
            if (candidate.checksum) result.checksum = hex(bytes.subarray(offset, offset + 2));
            else result.warnings.push('最終営業はファイル末尾までの長さで判定しました。チェックサム領域がありません。');
            break;
        }
    }
    return result;
}

export function dailyReportRows(bytes: Uint8Array, record?: DailyReportRecord, stopIndex = 0): DailyReportRow[] {
    if (record && (!Number.isInteger(stopIndex) || stopIndex < 0 || stopIndex > record.stopCount)) {
        throw new Error('立寄の表示位置が不正です。');
    }
    if (record && (record.offset < 3 || record.offset + record.length > bytes.length
        || record.length !== dailyReportBaseLength + record.stopCount * dailyReportStopLength)) {
        throw new Error('営業データの範囲が不正です。');
    }
    let position = record?.offset ?? 0;
    return dailyReportFields.map((field, index) => {
        // 乗車 → 立寄1～3 → 降車 → 人数・料金以降。
        if (index === 4 && record) position += record.stopCount * dailyReportStopLength;
        let source = position;
        if (record && stopIndex > 0 && index >= 4 && index <= 6) {
            source -= (record.stopCount - stopIndex + 1) * dailyReportStopLength;
        }
        const content = record ? hex(bytes.subarray(source, source + field.size)) : '';
        const row = {
            ...field,
            name: stopIndex > 0 && index >= 4 && index <= 6 ? field.name.replace('降車', `立寄${stopIndex}`) : field.name,
            content,
            detail: record ? formatDetail(field, content) : '',
        };
        position += field.size;
        return row;
    });
}

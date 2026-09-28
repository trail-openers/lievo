import { readFileSync } from 'fs';
import path from 'path';

function processItems(items) {
    return items.filter(x => x > 0).map(x => x * 2);
}

function loadConfig(filePath) {
    try {
        const data = readFileSync(filePath, 'utf8');
        return data;
    } catch (e) {
        return null;
    }
}

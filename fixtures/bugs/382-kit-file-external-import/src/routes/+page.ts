import { db } from '../../../_shared/kit-external-db';
export const load = (e) => ({ n: db.n, u: e.url.nope });

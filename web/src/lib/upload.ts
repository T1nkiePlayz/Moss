// Abortable multipart upload with progress (XHR: fetch has no upload progress). Lazy-loaded with the modpack UI.
import { ApiError, apiUrl, authToken, isMock, send, type ModpackInspect } from './api';

export interface Upload<T> { promise: Promise<T>; abort(): void }
export type Progress = (loaded: number, total: number) => void;

export function uploadForm<T>(path: string, fd: FormData, onProgress?: Progress): Upload<T> {
  const aborted = () => new ApiError(0, 'upload.aborted', 'Upload cancelled');
  if (isMock()) {
    // Dev mock: fake a steady upload, then hand the form to the in-memory backend.
    const total = [...fd.values()].reduce<number>((n, v) => n + (v instanceof File ? v.size : 0), 0) || 1;
    let stop = () => {};
    const promise = new Promise<T>((resolve, reject) => {
      let loaded = 0;
      const iv = setInterval(async () => {
        loaded = Math.min(total, loaded + Math.max(total / 12, 1));
        onProgress?.(loaded, total);
        if (loaded >= total) {
          clearInterval(iv);
          try { resolve(await send<T>('POST', path, fd)); } catch (e) { reject(e); }
        }
      }, 160);
      stop = () => { clearInterval(iv); reject(aborted()); };
    });
    return { promise, abort: () => stop() };
  }
  const xhr = new XMLHttpRequest();
  const promise = new Promise<T>((resolve, reject) => {
    authToken().then((tok) => {
      xhr.open('POST', apiUrl(path));
      if (tok) xhr.setRequestHeader('Authorization', `Bearer ${tok}`);
      xhr.responseType = 'text';
      xhr.upload.onprogress = (ev) => ev.lengthComputable && onProgress?.(ev.loaded, ev.total);
      xhr.onload = () => {
        let j: any = null;
        try { j = xhr.responseText ? JSON.parse(xhr.responseText) : null; } catch { /* non-JSON body */ }
        if (xhr.status >= 200 && xhr.status < 300) return resolve(j as T);
        const er = j?.error ?? {};
        reject(new ApiError(xhr.status, er.code ?? `http.${xhr.status}`, er.message ?? (xhr.statusText || 'Upload failed'), er.fields ?? [], er.request_id));
      };
      xhr.onerror = () => reject(new ApiError(0, 'network', 'The upload was interrupted. Check your connection and try again.'));
      xhr.onabort = () => reject(aborted());
      xhr.send(fd);
    }, reject);
  });
  return { promise, abort: () => xhr.abort() };
}

export function inspectModpackFile(file: File, onProgress?: Progress): Upload<ModpackInspect> {
  const fd = new FormData();
  fd.append('file', file, file.name);
  return uploadForm<ModpackInspect>('/modpacks/inspect', fd, onProgress);
}

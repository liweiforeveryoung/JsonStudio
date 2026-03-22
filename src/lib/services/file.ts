// File operation service - communicates with Rust backend
import { invoke } from '@tauri-apps/api/core';

/**
 * Open file using file picker dialog
 * @returns Tuple of [filePath, content] or null if cancelled
 */
export async function openFileDialog(): Promise<[string, string] | null> {
  const result = await invoke<[string, string] | null>('open_file_dialog');
  return result;
}

/**
 * Save content to existing file path
 */
export async function saveFile(path: string, content: string): Promise<void> {
  await invoke('save_file', { path, content });
}

/**
 * Save content to new file using save dialog
 * @returns File path or null if cancelled
 */
export async function saveFileDialog(content: string, defaultFileName?: string): Promise<string | null> {
  // Tauri command args are deserialized from camelCase keys
  const payload: { content: string; defaultFileName?: string } = { content };
  if (defaultFileName) payload.defaultFileName = defaultFileName;
  const result = await invoke<string | null>('save_file_dialog', payload);
  return result;
}

/**
 * Read file content by path (for drag & drop)
 */
export async function readFile(path: string): Promise<string> {
  return await invoke<string>('read_file', { path });
}

/**
 * Check if file path is valid JSON file
 */
export async function isJsonFile(path: string): Promise<boolean> {
  return await invoke<boolean>('is_json_file', { path });
}

/**
 * Get file name from path
 */
export async function getFileName(path: string): Promise<string | null> {
  return await invoke<string | null>('get_file_name', { path });
}

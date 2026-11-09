import { configDefaults, defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    exclude: [...configDefaults.exclude, 'tests/vendor/**'],
    coverage: {
      provider: 'v8',
      include: ['js/**'],
      reporter: ['text'],
    },
  },
});

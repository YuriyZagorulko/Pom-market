FROM node:20-bookworm-slim
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY packages ./packages
COPY apps/user ./apps/user
RUN pnpm install --frozen-lockfile --filter @pom/user...
ARG API_INTERNAL_URL=http://api:4321
ENV API_INTERNAL_URL=$API_INTERNAL_URL
RUN pnpm --filter @pom/user build
WORKDIR /app/apps/user
EXPOSE 4323
CMD ["pnpm", "start"]

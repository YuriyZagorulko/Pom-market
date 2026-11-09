FROM node:20-bookworm-slim
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY packages ./packages
COPY apps/admin ./apps/admin
RUN pnpm install --frozen-lockfile --filter @pom/admin...
ARG API_INTERNAL_URL=http://api:4321
ENV API_INTERNAL_URL=$API_INTERNAL_URL
RUN pnpm --filter @pom/admin build
WORKDIR /app/apps/admin
EXPOSE 4322
CMD ["pnpm", "start"]

/// <reference path="./.sst/platform/config.d.ts" />

export default $config({
  app(input) {
    return {
      name: "cranbrook-catering-api",
      removal: input?.stage === "production" ? "retain" : "remove",
      protect: ["production"].includes(input?.stage),
      home: "aws",
    };
  },
  async run() {
    new sst.aws.Function("main", {
      url: true,
      runtime: "rust",
      handler: ".main",
      architecture: "arm64",
    });
  },
});

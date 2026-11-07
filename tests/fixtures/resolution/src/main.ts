import { util } from "./util";
import type { Thing } from "./types";
import { lib } from "@lib";
import { helper } from "@app/helper";
import { workspace } from "@fixture/lib";
import { libUtil } from "@fixture/lib/util";
import React from "react";
import missing from "./missing";
import data from "./data.json";
import "@app/missing";
import "./side-effect";
export { thing } from "./reexport";

const lazy = () => import("./lazy");
const unknownPath = "./lazy";
const dynamic = () => import(unknownPath);
const legacy = require("./legacy");

export const run = [util, lib, helper, workspace, libUtil, React, missing, data, lazy, dynamic, legacy];

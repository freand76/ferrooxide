// included by include_lines.v
`define INC_SUM(a, b) ((a) + (b))
task inc_task;
  $info("in the included file, line 4");
endtask

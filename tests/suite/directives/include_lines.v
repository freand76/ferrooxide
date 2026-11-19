// expected
// Locations after an `include (with a tab before the name) and after a macro call whose
// arguments span lines: messages name the file and line they come from.
module m;
`include	"include_lines.vh"
  integer s;
  initial begin
    $info("line 8");
    s = `INC_SUM(1,
                 2);
    $info("line 11, s = %0d", s);
    inc_task;
  end
endmodule

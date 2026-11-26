// An escaped identifier (IEEE 1800-2017 5.6.1) runs to the next white space: a backtick,
// quote or comment start in it is part of the name, not a macro, string or comment.
`define W 4
module t;
  reg [`W-1:0] \`W"x//y ;
  reg \a/*b ;
  initial begin
    \`W"x//y = 4'h9;
    \a/*b = 1;
    if (\`W"x//y === 4'h9 && \a/*b === 1'b1 && $bits(\`W"x//y ) == `W) $display("PASSED");
    else $display("FAILED");
  end
endmodule

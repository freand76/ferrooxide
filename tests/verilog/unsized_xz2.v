// unsized x/z constants as task and function arguments
module t;
  reg [63:0] q; reg [39:0] p;
  task tk(input [63:0] v); $display("task %h", v); endtask
  function [63:0] f(input [63:0] v); f = v; endfunction
  initial begin
    q = 'bx; $display("%h", q);
    q = 'bz; $display("%h", q);
    p = 'hx; $display("%h", p);
    tk('bx); tk('hz); tk(8'bx);
    q = f('bx); $display("f %h", q);
    if (q !== 'bx) $display("ne"); else $display("eq");
    $display("%b", 'bx === 64'bx);
  end
endmodule

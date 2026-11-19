// A case statement with a real expression or a real item compares all values as reals,
// each converted with its own signedness (IEEE 1364-2005 9.5).
module test;
  function [2:0] by_real(input real v);
    case (v)
      4'b0001: by_real = 1;
      3'sb010: by_real = 2;
      2'sb11:  by_real = 3;
      default: by_real = 4;
    endcase
  endfunction
  function [2:0] real_item(input signed [2:0] v);
    case (v)
      4'b0110: real_item = 1;
      3'sb110: real_item = 2;
      -1.0:    real_item = 3;
      default: real_item = 4;
    endcase
  endfunction
  real r;
  initial begin
    $display("%0d %0d %0d %0d", by_real(1.0), by_real(2.0), by_real(-1.0), by_real(1.5));
    $display("%0d %0d %0d", real_item(3'sb110), real_item(3'sb111), real_item(3'sb011));
    r = 2.5;
    case (r)
      2: $display("int 2");
      2.5: $display("real 2.5");
      default: $display("none");
    endcase
  end
endmodule

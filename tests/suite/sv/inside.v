// inside (IEEE 1800 11.4.13): values compare with ==? (x/z in an item are wildcards), [lo:hi]
// ranges ($ an open bound); 1 if any item matches, else x if a comparison is x, else 0.
// case ... inside (12.5.4) matches the same way.
module t;
  integer errors = 0;
  reg [7:0] b;
  reg [3:0] n;
  reg [8*5:1] s;
  task check(input integer line, input got, input want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got, want);
      errors = errors + 1;
    end
  endtask
  function [8*5:1] kind(input [3:0] v);
    case (v) inside
      [0:3]: kind = "low";
      4'b01??, 9: kind = "mid";
      default: kind = "other";
    endcase
  endfunction
  initial begin
    b = 5;
    check(`__LINE__, b inside {1, [4:6]}, 1);
    check(`__LINE__, b inside {7}, 0);
    check(`__LINE__, b inside {[$:5]}, 1);
    check(`__LINE__, b inside {[6:$]}, 0);
    check(`__LINE__, b inside {8'b0000_01?1}, 1);
    check(`__LINE__, b inside {8'b0000_0x00}, 0);
    b = 8'bx;
    check(`__LINE__, b inside {1, 2}, 1'bx);
    check(`__LINE__, b inside {8'bx}, 1);       // all bits wildcards
    b = 3;
    check(`__LINE__, b + 1 inside {4}, 1);      // binds like a relational operator
    check(`__LINE__, !(b inside {3}), 0);
    if (kind(2) != "low" || kind(5) != "mid" || kind(9) != "mid" || kind(8) != "other") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    if (errors == 0) $display("PASSED");
  end
endmodule

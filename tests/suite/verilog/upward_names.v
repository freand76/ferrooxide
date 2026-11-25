// expected
// Hierarchical names searched upwards through the instance hierarchy (a generate block,
// an instance of an enclosing module, an enclosing instance by its module name), and
// generate conditionals directly nested in a case item or an if branch, whose named
// blocks belong to the enclosing scope.
module leaf;
  reg [7:0] own;
  initial begin
    #1 gb.t;
    gb.x = 8'h5a;
    mid.v = 3;
    own = 1;
    #1 $display("%m: gb.x=%h mid.v=%0d inner.own=%0d", gb.x, mid.v, inner.own);
  end
endmodule

module mid;
  integer v;
  leaf inner();
endmodule

module outer #(parameter P = 1);
  mid m();
  generate
    begin : gb
      reg [7:0] x;
      task t;
        $display("%m called");
      endtask
    end
    case (P)
      1, 2: if (P == 1) begin : blk
              integer k = 11;
            end else begin : blk
              integer k = 22;
            end
      default: begin : blk
                 integer k = 33;
               end
    endcase
    if (P > 1)
      if (P > 2) begin : deep
        integer d = 3;
      end else begin : deep
        integer d = 2;
      end
  endgenerate
  initial #3 $display("%m: blk.k=%0d", blk.k);
endmodule

module top;
  outer #(1) o1();
  outer #(2) o2();
  outer #(3) o3();
  initial #4 $display("deep %0d %0d", o2.deep.d, o3.deep.d);
endmodule
